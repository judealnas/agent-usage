pub mod cli_integration;
mod entry_settings;
mod migration;
mod requests;
#[allow(dead_code)] // Legacy schema retained for migration and its regression tests.
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use agent_usage_core::options::CODEX_EFFORTS;
use agent_usage_core::{Provider, UsageReport};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use entry_settings::{Configuration, EntryDraft, Preferences, Settings};

/// Event carrying one finished `UsageReport` to the frontend.
const REPORT_EVENT: &str = "usage-report";

struct AppState {
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    /// Providers with a fetch running. At most one fetch per provider.
    in_flight: Mutex<requests::Requests>,
    /// Bumped whenever a provider's timer is restarted, so that the timer
    /// task from before the restart notices and stops.
    timer_generations: Mutex<HashMap<String, u64>>,
}

impl AppState {
    fn snapshot(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Apply a change, save it to disk, and return the new settings. The
    /// change is installed in memory only after saving succeeds.
    fn update(
        &self,
        change: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<Settings, String> {
        let mut settings = self.settings.lock().unwrap();
        let mut next = settings.clone();
        change(&mut next)?;
        next.save(&self.settings_path)?;
        *settings = next.clone();
        Ok(next)
    }
}

#[derive(Clone, Serialize)]
struct EntryReport {
    entry_id: String,
    generation: u64,
    report: Option<UsageReport>,
    source_report: Option<agent_usage_core::sources::SourceReport>,
}

fn spawn_fetch(app: AppHandle, provider: String) {
    tauri::async_runtime::spawn(fetch_now(app, provider));
}

async fn fetch_now(app: AppHandle, provider: String) {
    let state = app.state::<AppState>();
    let Some(entry) = state.snapshot().provider(&provider).cloned() else {
        return;
    };
    if !state
        .in_flight
        .lock()
        .unwrap()
        .begin(&provider, entry.generation)
    {
        return;
    }
    let generation = entry.generation;
    let config = entry.config.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        use agent_usage_core::{
            codex_account::AccountOptions,
            sources::{self, CodexSource},
            CodexOptions,
        };
        match config {
            Configuration::Claude { options } => {
                let report = agent_usage_core::claude::fetch(&options)
                    .unwrap_or_else(|e| UsageReport::failed(Provider::Claude, e));
                (Some(report), None)
            }
            Configuration::CodexInference { options } => (
                None,
                Some(sources::fetch(
                    CodexSource::Inference,
                    &options,
                    &AccountOptions::default(),
                )),
            ),
            Configuration::CodexAccount { options } => (
                None,
                Some(sources::fetch(
                    CodexSource::Account,
                    &CodexOptions::default(),
                    &options,
                )),
            ),
        }
    })
    .await;
    let retry = state.in_flight.lock().unwrap().finish(&provider);
    if retry {
        spawn_fetch(app.clone(), provider.clone());
    }
    // Hold settings through emit so an edit/removal cannot publish an obsolete generation.
    let settings = state.settings.lock().unwrap();
    if !settings
        .provider(&provider)
        .is_some_and(|p| p.generation == generation)
    {
        return;
    }
    let (report, source_report) = result.unwrap_or_else(|_| {
        (
            Some(UsageReport::failed(
                Provider::Codex,
                "The query crashed unexpectedly.",
            )),
            None,
        )
    });
    let _ = app.emit(
        REPORT_EVENT,
        EntryReport {
            entry_id: provider,
            generation,
            report,
            source_report,
        },
    );
}

/// Stop the provider's current timer, and start a new one if the provider
/// is shown and set to refresh automatically.
fn restart_timer(app: &AppHandle, provider: String) {
    let state = app.state::<AppState>();
    let generation = {
        let mut generations = state.timer_generations.lock().unwrap();
        let generation = generations.entry(provider.clone()).or_insert(0);
        *generation += 1;
        *generation
    };

    let settings = state.snapshot();
    let Some(config) = settings.provider(&provider) else {
        return;
    };
    if !config.shown || !config.auto_refresh {
        return;
    }
    let interval = Duration::from_secs(config.interval_seconds.max(1));

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let current = app
                .state::<AppState>()
                .timer_generations
                .lock()
                .unwrap()
                .get(&provider)
                .copied();
            if current != Some(generation) {
                break;
            }
            fetch_now(app.clone(), provider.clone()).await;
        }
    });
}

/// Fetch a provider now and restart its timer, unless it is already being
/// fetched, in which case nothing happens.
fn refresh_provider(app: &AppHandle, provider: String) {
    restart_timer(app, provider.clone());
    tauri::async_runtime::spawn(fetch_now(app.clone(), provider));
}

#[derive(Serialize)]
struct ProviderInfo {
    id: Provider,
    name: &'static str,
}

#[derive(Serialize)]
struct Meta {
    author: &'static str,
    version: &'static str,
    repository: &'static str,
    providers: Vec<ProviderInfo>,
    codex_efforts: Vec<&'static str>,
}

#[tauri::command]
fn get_meta() -> Meta {
    Meta {
        author: env!("CARGO_PKG_AUTHORS"),
        version: env!("CARGO_PKG_VERSION"),
        repository: env!("CARGO_PKG_REPOSITORY"),
        providers: Provider::ALL
            .into_iter()
            .map(|id| ProviderInfo {
                id,
                name: id.display_name(),
            })
            .collect(),
        codex_efforts: CODEX_EFFORTS.to_vec(),
    }
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.snapshot()
}

/// Open only this application's repository, never a frontend-supplied command or URL.
#[tauri::command]
fn open_repository() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    let mut child = command
        .arg(env!("CARGO_PKG_REPOSITORY"))
        .spawn()
        .map_err(|error| format!("Could not open the repository: {error}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Called by the frontend once it is listening: fetch every shown provider
/// and start the timers.
#[tauri::command]
fn start(app: AppHandle, state: State<AppState>) {
    for provider in state.snapshot().shown() {
        refresh_provider(&app, provider);
    }
}

#[tauri::command]
fn refresh(app: AppHandle, provider: String) {
    refresh_provider(&app, provider);
}

#[tauri::command]
fn refresh_all(app: AppHandle, state: State<AppState>) {
    for provider in state.snapshot().shown() {
        refresh_provider(&app, provider);
    }
}

#[tauri::command]
fn set_auto_refresh(
    app: AppHandle,
    state: State<AppState>,
    provider: String,
    auto_refresh: bool,
    interval_seconds: u64,
) -> Result<Settings, String> {
    let result = state.update(|s| s.set_auto_refresh(&provider, auto_refresh, interval_seconds));
    restart_timer(&app, provider);
    result
}

#[tauri::command]
fn set_order(state: State<AppState>, order: Vec<String>) -> Result<Settings, String> {
    state.update(|s| {
        s.set_order(&order);
        Ok(())
    })
}

#[tauri::command]
fn set_shown(
    app: AppHandle,
    state: State<AppState>,
    provider: String,
    shown: bool,
) -> Result<Settings, String> {
    let result = state.update(|s| s.set_shown(&provider, shown));
    if shown {
        refresh_provider(&app, provider);
    } else {
        restart_timer(&app, provider);
    }
    result
}

#[tauri::command]
fn set_pinned(app: AppHandle, state: State<AppState>, pinned: bool) -> Result<Settings, String> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_always_on_top(pinned)
            .map_err(|err| err.to_string())?;
    }
    state.update(|s| {
        s.pinned = pinned;
        Ok(())
    })
}

#[tauri::command]
fn save_preferences(state: State<AppState>, preferences: Preferences) -> Result<Settings, String> {
    state.update(|s| s.apply_preferences(preferences))
}

#[tauri::command]
fn save_entry(
    app: AppHandle,
    state: State<AppState>,
    entry: EntryDraft,
) -> Result<Settings, String> {
    let before = entry
        .id
        .as_ref()
        .and_then(|id| state.snapshot().provider(id).cloned());
    let mut saved_id = String::new();
    let settings = state.update(|s| {
        saved_id = s.upsert(entry)?;
        Ok(())
    })?;
    let saved = settings.provider(&saved_id).unwrap();
    if before.as_ref().is_none_or(|old| old.config != saved.config) {
        restart_timer(&app, saved_id.clone());
    }
    Ok(settings)
}

#[tauri::command]
fn remove_entry(app: AppHandle, state: State<AppState>, id: String) -> Result<Settings, String> {
    let settings = state.update(|s| s.remove(&id))?;
    restart_timer(&app, id);
    Ok(settings)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    migration::migrate(&context.config().identifier)
        .expect("Could not migrate existing Agent Usage settings; original files were preserved");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let settings_path = Settings::default_path();
            let settings = Settings::load(&settings_path).map_err(std::io::Error::other)?;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(settings.pinned);
            }
            app.manage(AppState {
                settings_path,
                settings: Mutex::new(settings),
                in_flight: Mutex::new(requests::Requests::default()),
                timer_generations: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_meta,
            get_cli_status,
            set_cli_enabled,
            open_repository,
            get_settings,
            start,
            refresh,
            refresh_all,
            set_auto_refresh,
            set_order,
            set_shown,
            set_pinned,
            save_preferences,
            save_entry,
            remove_entry,
        ])
        .run(context)
        .expect("error while running the application");
}

#[tauri::command]
fn get_cli_status() -> Result<cli_integration::CliStatus, String> {
    cli_integration::status()
}

#[tauri::command]
fn set_cli_enabled(enabled: bool) -> Result<cli_integration::CliStatus, String> {
    cli_integration::set_enabled(enabled, false)
}
