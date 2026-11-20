//! Native tray state. All icon mutations run on the main thread; polling stays in Rust.
use crate::{
    entry_settings::{Configuration, Entry},
    AppState, EntryReport,
};
use agent_usage_core::Metric;
use std::collections::HashMap;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

#[derive(Default)]
pub struct TrayState {
    icons: HashMap<String, TrayIcon>,
    rendered: HashMap<String, (Vec<u8>, String)>,
}
impl TrayState {
    pub fn available(&self) -> bool {
        !self.icons.is_empty()
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
pub fn schedule(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || sync(&handle)) {
        eprintln!("Could not schedule tray update: {error}");
    }
}

fn create(app: &AppHandle, id: &str, image: Image<'_>, tooltip: &str) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, "tray-open", "Open Agent Usage", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open])?;
    if id != "app" {
        menu.append(&MenuItem::with_id(
            app,
            format!("tray-refresh:{id}"),
            "Refresh this entry",
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&quit)?;
    TrayIconBuilder::with_id(format!("usage-{id}"))
        .icon(image)
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_window(tray.app_handle());
            }
        })
        .build(app)
}

pub fn sync(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.snapshot();
    let reports = state.reports.lock().unwrap().clone();
    let now = chrono::Utc::now().timestamp();
    let mut tray = state.tray.lock().unwrap();
    let entries: Vec<_> = settings
        .providers
        .iter()
        .filter(|p| p.tray_enabled)
        .collect();
    let wanted: Vec<String> = if entries.is_empty() {
        vec!["app".into()]
    } else {
        entries.iter().map(|p| p.id.clone()).collect()
    };
    // Create replacements before removing old icons, preserving an escape route on failure.
    for id in &wanted {
        let result = if let Some(entry) = entries.iter().find(|p| &p.id == id) {
            let data = indicator(entry, reports.get(id), now);
            render(entry, &data, settings.thresholds.critical).map(|pixels| (pixels, data.tooltip))
        } else {
            app.default_window_icon()
                .map(|icon| (icon.rgba().to_vec(), "Agent Usage".to_string()))
                .ok_or_else(|| "Missing app icon".to_string())
        };
        let result = result.and_then(|(pixels, tooltip)| {
            if tray.rendered.get(id) == Some(&(pixels.clone(), tooltip.clone())) {
                return Ok(());
            }
            let (width, height) = if id == "app" {
                let icon = app.default_window_icon().unwrap();
                (icon.width(), icon.height())
            } else {
                (32, 32)
            };
            let image = Image::new_owned(pixels.clone(), width, height);
            if let Some(icon) = tray.icons.get(id) {
                icon.set_icon(Some(image)).map_err(|e| e.to_string())?;
                icon.set_tooltip(Some(&tooltip))
                    .map_err(|e| e.to_string())?;
            } else {
                let icon = create(app, id, image, &tooltip).map_err(|e| e.to_string())?;
                tray.icons.insert(id.clone(), icon);
            }
            tray.rendered.insert(id.clone(), (pixels, tooltip));
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("Could not update tray: {error}");
        }
    }
    if wanted.iter().all(|id| tray.icons.contains_key(id)) {
        let obsolete: Vec<_> = tray
            .icons
            .keys()
            .filter(|id| !wanted.contains(id))
            .cloned()
            .collect();
        for id in obsolete {
            app.remove_tray_by_id(&format!("usage-{id}"));
            tray.icons.remove(&id);
            tray.rendered.remove(&id);
        }
    }
    if !tray.available() {
        show_window(app);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ring {
    remaining: f64,
    used: f64,
    reset: Option<i64>,
}
struct Indicator {
    rings: [Option<Ring>; 2],
    tooltip: String,
}

fn shorten(text: &str, max: usize) -> String {
    let mut units = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}

fn indicator(entry: &Entry, report: Option<&EntryReport>, now: i64) -> Indicator {
    let mut rings = [None, None];
    let mut status = "Waiting for usage";
    let mut group_name = None;
    let mut fetched_at = None;
    let mut metrics: &[Metric] = &[];
    if let Some(report) = report.filter(|r| r.generation == entry.generation) {
        if let Some(source) = &report.source_report {
            fetched_at = Some(source.fetched_at);
            if source.error.is_some() {
                status = "Refresh failed";
            } else {
                let group = match &entry.tray_group {
                    Some(key) => source.groups.iter().find(|g| &g.key == key),
                    None if source.groups.len() == 1 => source.groups.first(),
                    _ => None,
                };
                if let Some(group) = group {
                    group_name = Some(
                        group
                            .limit_name
                            .as_deref()
                            .or(group.limit_id.as_deref())
                            .unwrap_or(&group.key),
                    );
                    if group.error.is_some() {
                        status = "Refresh failed";
                    } else {
                        metrics = &group.metrics;
                        status = "";
                    }
                } else if entry.tray_group.is_some() {
                    status = "Selected group unavailable";
                } else {
                    status = "Choose a quota group in Settings";
                }
            }
        } else if let Some(report) = &report.report {
            fetched_at = Some(report.fetched_at);
            if report.error.is_some() {
                status = "Refresh failed";
            } else {
                metrics = &report.metrics;
                status = "";
            }
        }
    }
    let mut lines = Vec::new();
    for (index, duration) in [300, 10080].into_iter().enumerate() {
        let label = if index == 0 { "5h" } else { "7d" };
        let metric = metrics.iter().find(
            |m| matches!(m, Metric::Window { duration_minutes: Some(d), .. } if *d == duration),
        );
        let detail = match metric {
            Some(Metric::Window {
                remaining_percent,
                used_percent,
                resets_at,
                ..
            }) if remaining_percent.is_finite() && used_percent.is_finite() => {
                if resets_at.is_some_and(|reset| reset <= now) {
                    "reset passed; awaiting refresh".into()
                } else {
                    let ring = Ring {
                        remaining: remaining_percent.clamp(0.0, 100.0),
                        used: *used_percent,
                        reset: *resets_at,
                    };
                    rings[index] = Some(ring);
                    let reset = ring
                        .reset
                        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                        .map(|t| {
                            t.with_timezone(&chrono::Local)
                                .format("%a %H:%M")
                                .to_string()
                        });
                    format!(
                        "{:.0}% left{}",
                        ring.remaining,
                        reset.map(|t| format!("; resets {t}")).unwrap_or_default()
                    )
                }
            }
            _ => "unavailable".into(),
        };
        lines.push(format!("{label}: {detail}"));
    }
    let title = match group_name {
        Some(group) => format!("{} / {}", shorten(entry.label(), 15), shorten(group, 10)),
        None => shorten(entry.label(), 28),
    };
    let mut tooltip = format!(
        "{title}\n{}",
        if status.is_empty() {
            lines.join("\n")
        } else {
            status.into()
        }
    );
    if !entry.auto_refresh {
        let time = fetched_at
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_else(|| "never".into());
        tooltip.push_str(&format!("\nManual; updated {time}"));
    }
    // Windows szTip is 128 UTF-16 units including the null terminator.
    let mut units = 0;
    tooltip = tooltip
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 127
        })
        .collect();
    Indicator { rings, tooltip }
}

fn color(used: f64, threshold: f64) -> String {
    let hue = 120.0 * (1.0 - (used / threshold).clamp(0.0, 1.0));
    // HSL(hue, 65%, 50%): same hue progression as colors.ts, with neutral tray lightness.
    let c = 0.65;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g) = if hue < 60.0 { (c, x) } else { (x, c) };
    format!(
        "#{:02x}{:02x}{:02x}",
        ((r + 0.175) * 255.0).round() as u8,
        ((g + 0.175) * 255.0).round() as u8,
        (0.175_f64 * 255.0).round() as u8
    )
}

fn render(entry: &Entry, data: &Indicator, threshold: f64) -> Result<Vec<u8>, String> {
    let mut svg = String::from(
        "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32' viewBox='0 0 32 32'>",
    );
    for (radius, ring) in [14.5, 10.5].into_iter().zip(data.rings) {
        let track = if ring.is_some_and(|r| r.remaining == 0.0) {
            color(100.0, threshold)
        } else {
            "#7c828b".into()
        };
        svg.push_str(&format!("<circle cx='16' cy='16' r='{radius}' fill='none' stroke='{track}' stroke-width='2.5'/>"));
        if let Some(ring) = ring.filter(|r| r.remaining > 0.0) {
            let length = std::f64::consts::TAU * radius;
            svg.push_str(&format!("<circle cx='16' cy='16' r='{radius}' fill='none' stroke='{}' stroke-width='2.5' stroke-dasharray='{} {}' transform='rotate(-90 16 16)'/>", color(ring.used, threshold), length * ring.remaining / 100.0, length));
        }
    }
    // A white center disk keeps both existing provider marks readable on either taskbar theme.
    svg.push_str("<circle cx='16' cy='16' r='7.75' fill='white'/>");
    let logo = if matches!(entry.config, Configuration::Claude { .. }) {
        include_str!("../../src/assets/claude.svg").replace(
            "viewBox=\"0 0 24 24\"",
            "x=\"9\" y=\"9\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\"",
        )
    } else {
        include_str!("../../src/assets/openai.svg").replace(
            "width=\"721\" height=\"721\" viewBox=\"0 0 721 721\"",
            "x=\"9\" y=\"9\" width=\"14\" height=\"14\" viewBox=\"118 119 485 481\"",
        )
    };
    svg.push_str(&logo);
    svg.push_str("</svg>");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(32, 32).ok_or("Could not allocate tray image")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    // tiny-skia uses premultiplied alpha; Tauri expects straight RGBA.
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        if p[3] > 0 {
            for i in 0..3 {
                p[i] = ((u32::from(p[i]) * 255) / u32::from(p[3])).min(255) as u8;
            }
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_usage_core::{Provider, UsageReport};
    #[test]
    fn rings_use_duration_not_order_and_expire_independently() {
        let entry = crate::entry_settings::Settings::default()
            .providers
            .remove(0);
        let mut report = UsageReport::new(Provider::Codex);
        report.metrics = vec![
            Metric::window("weekly", Some(10080), 70.0, Some(200)),
            Metric::window("session", Some(300), 20.0, Some(100)),
        ];
        let payload = EntryReport {
            entry_id: entry.id.clone(),
            generation: 0,
            report: Some(report),
            source_report: None,
        };
        let data = indicator(&entry, Some(&payload), 50);
        assert_eq!(data.rings.map(|r| r.unwrap().remaining), [80.0, 30.0]);
        let expired = indicator(&entry, Some(&payload), 100);
        assert!(expired.rings[0].is_none());
        assert!(expired.rings[1].is_some());
        assert_eq!(render(&entry, &data, 90.0).unwrap().len(), 4096);
    }
    #[test]
    fn obsolete_generation_is_never_displayed() {
        let entry = crate::entry_settings::Settings::default()
            .providers
            .remove(0);
        let payload = EntryReport {
            entry_id: entry.id.clone(),
            generation: 99,
            report: Some(UsageReport::new(Provider::Codex)),
            source_report: None,
        };
        assert!(indicator(&entry, Some(&payload), 0)
            .tooltip
            .contains("Waiting"));
    }
    #[test]
    fn color_matches_hue_endpoints() {
        assert_eq!(color(0.0, 90.0), "#2dd22d");
        assert_eq!(color(90.0, 90.0), "#d22d2d");
        assert_eq!(color(120.0, 90.0), color(90.0, 90.0));
    }

    #[test]
    fn group_selection_never_combines_accounts_and_errors_clear_rings() {
        use agent_usage_core::sources::SourceReport;
        let mut entry = crate::entry_settings::Settings::default()
            .providers
            .remove(0);
        let mut report = UsageReport::new(Provider::Codex);
        report.metrics = vec![Metric::window("5h", Some(300), 20.0, None)];
        let mut source = SourceReport::inference(report, "test");
        let mut second = source.groups[0].clone();
        second.key = "second".into();
        second.metrics = vec![Metric::window("7d", Some(10080), 70.0, None)];
        source.groups.push(second);
        let mut payload = EntryReport {
            entry_id: entry.id.clone(),
            generation: 0,
            report: None,
            source_report: Some(source),
        };
        assert!(indicator(&entry, Some(&payload), 0)
            .tooltip
            .contains("Choose a quota group"));
        entry.tray_group = Some("second".into());
        let selected = indicator(&entry, Some(&payload), 0);
        assert!(selected.rings[0].is_none());
        assert_eq!(selected.rings[1].unwrap().remaining, 30.0);
        payload.source_report.as_mut().unwrap().groups[1].error = Some("offline".into());
        let failed = indicator(&entry, Some(&payload), 0);
        assert_eq!(failed.rings, [None, None]);
        assert!(failed.tooltip.contains("Refresh failed"));
        entry.tray_group = Some("removed".into());
        assert!(indicator(&entry, Some(&payload), 0)
            .tooltip
            .contains("unavailable"));
    }

    #[test]
    fn tooltip_preserves_both_windows_with_long_unicode_names() {
        let mut entry = crate::entry_settings::Settings::default()
            .providers
            .remove(0);
        entry.name = Some("😀".repeat(100));
        entry.auto_refresh = false;
        let mut report = UsageReport::new(Provider::Codex);
        report.metrics = vec![
            Metric::window("5h", Some(300), 20.0, Some(500)),
            Metric::window("7d", Some(10080), 70.0, Some(900)),
        ];
        let payload = EntryReport {
            entry_id: entry.id.clone(),
            generation: 0,
            report: Some(report),
            source_report: None,
        };
        let text = indicator(&entry, Some(&payload), 0).tooltip;
        assert!(text.encode_utf16().count() <= 127);
        assert!(text.contains("5h: 80%"));
        assert!(text.contains("7d: 30%"));
        assert!(text.contains("Manual; updated"));
    }

    /// Optional visual artifact from the production renderer, without live credentials or polling.
    #[test]
    #[ignore]
    fn export_tray_preview() {
        use resvg::tiny_skia::{Color, FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
        let mut sheet = Pixmap::new(512, 256).unwrap();
        let entries = crate::entry_settings::Settings::default().providers;
        for row in 0..4 {
            let background = if row % 2 == 0 {
                Color::from_rgba8(240, 240, 240, 255)
            } else {
                Color::from_rgba8(30, 30, 30, 255)
            };
            let mut strip = Pixmap::new(512, 64).unwrap();
            strip.fill(background);
            sheet.draw_pixmap(
                0,
                row * 64,
                strip.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            for (column, values) in [
                [Some(100.0), Some(100.0)],
                [Some(75.0), Some(30.0)],
                [Some(5.0), Some(0.0)],
                [None, None],
            ]
            .into_iter()
            .enumerate()
            {
                let data = Indicator {
                    rings: values.map(|v| {
                        v.map(|remaining| Ring {
                            remaining,
                            used: 100.0 - remaining,
                            reset: None,
                        })
                    }),
                    tooltip: String::new(),
                };
                let mut bytes = render(&entries[(row / 2) as usize], &data, 90.0).unwrap();
                for p in bytes.as_chunks_mut::<4>().0 {
                    for i in 0..3 {
                        p[i] = (u32::from(p[i]) * u32::from(p[3]) / 255) as u8;
                    }
                }
                let icon = Pixmap::from_vec(bytes, IntSize::from_wh(32, 32).unwrap()).unwrap();
                for (x, size) in [(4, 16), (26, 32), (66, 56)] {
                    let paint = PixmapPaint {
                        quality: FilterQuality::Bilinear,
                        ..Default::default()
                    };
                    sheet.draw_pixmap(
                        0,
                        0,
                        icon.as_ref(),
                        &paint,
                        Transform::from_row(
                            size as f32 / 32.0,
                            0.0,
                            0.0,
                            size as f32 / 32.0,
                            (column * 128 + x) as f32,
                            (row * 64 + (64 - size) / 2) as f32,
                        ),
                        None,
                    );
                }
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/tray-preview.png");
        sheet.save_png(&path).unwrap();
        println!("Preview: {}", path.display());
    }
}
