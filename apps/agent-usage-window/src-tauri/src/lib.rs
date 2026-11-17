mod settings;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use agent_usage_core::options::CODEX_EFFORTS;
use agent_usage_core::{Provider, UsageReport};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use settings::{Preferences, Settings};

/// Event carrying one finished `UsageReport` to the frontend.
const REPORT_EVENT: &str = "usage-report";

struct AppState {
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    /// Providers with a fetch running. At most one fetch per provider.
    in_flight: Mutex<HashSet<Provider>>,
    /// Bumped whenever a provider's timer is restarted, so that the timer
    /// task from before the restart notices and stops.
    timer_generations: Mutex<HashMap<Provider, u64>>,
}

impl AppState {
    fn snapshot(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Apply a change, save it to disk, and return the new settings. The
    /// change is kept in memory even if saving fails.
    fn update(
        &self,
        change: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<Settings, String> {
        let mut settings = self.settings.lock().unwrap();
        let mut next = settings.clone();
        change(&mut next)?;
        next.normalize();
        *settings = next.clone();
        next.save(&self.settings_path)?;
        Ok(next)
    }

    fn is_fetching(&self, provider: Provider) -> bool {
        self.in_flight.lock().unwrap().contains(&provider)
    }
}

/// Fetch one provider and send the report to the frontend. Does nothing if
/// that provider is already being fetched.
async fn fetch_now(app: AppHandle, provider: Provider) {
    let state = app.state::<AppState>();
    if !state.in_flight.lock().unwrap().insert(provider) {
        return;
    }

    let options = state.snapshot().options();
    // The library uses blocking HTTP, so keep it off the async runtime.
    let report =
        tauri::async_runtime::spawn_blocking(move || agent_usage_core::fetch(provider, &options))
            .await
            .unwrap_or_else(|_| UsageReport::failed(provider, "The query crashed unexpectedly."));

    state.in_flight.lock().unwrap().remove(&provider);
    let _ = app.emit(REPORT_EVENT, &report);
}

/// Stop the provider's current timer, and start a new one if the provider
/// is shown and set to refresh automatically.
fn restart_timer(app: &AppHandle, provider: Provider) {
    let state = app.state::<AppState>();
    let generation = {
        let mut generations = state.timer_generations.lock().unwrap();
        let generation = generations.entry(provider).or_insert(0);
        *generation += 1;
        *generation
    };

    let settings = state.snapshot();
    let Some(config) = settings.provider(provider) else {
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
            fetch_now(app.clone(), provider).await;
        }
    });
}

/// Fetch a provider now and restart its timer, unless it is already being
/// fetched, in which case nothing happens.
fn refresh_provider(app: &AppHandle, provider: Provider) {
    if app.state::<AppState>().is_fetching(provider) {
        return;
    }
    restart_timer(app, provider);
    tauri::async_runtime::spawn(fetch_now(app.clone(), provider));
}

#[derive(Serialize)]
struct ProviderInfo {
    id: Provider,
    name: &'static str,
}

#[derive(Serialize)]
struct Meta {
    providers: Vec<ProviderInfo>,
    codex_efforts: Vec<&'static str>,
}

#[tauri::command]
fn get_meta() -> Meta {
    Meta {
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

/// Called by the frontend once it is listening: fetch every shown provider
/// and start the timers.
#[tauri::command]
fn start(app: AppHandle, state: State<AppState>) {
    for provider in state.snapshot().shown() {
        refresh_provider(&app, provider);
    }
}

#[tauri::command]
fn refresh(app: AppHandle, provider: Provider) {
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
    provider: Provider,
    auto_refresh: bool,
    interval_seconds: u64,
) -> Result<Settings, String> {
    let result = state.update(|s| s.set_auto_refresh(provider, auto_refresh, interval_seconds));
    restart_timer(&app, provider);
    result
}

#[tauri::command]
fn set_order(state: State<AppState>, order: Vec<Provider>) -> Result<Settings, String> {
    state.update(|s| {
        s.set_order(&order);
        Ok(())
    })
}

#[tauri::command]
fn set_shown(
    app: AppHandle,
    state: State<AppState>,
    provider: Provider,
    shown: bool,
) -> Result<Settings, String> {
    let result = state.update(|s| s.set_shown(provider, shown));
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
fn save_preferences(
    app: AppHandle,
    state: State<AppState>,
    preferences: Preferences,
) -> Result<Settings, String> {
    let settings = state.update(|s| s.apply_preferences(preferences))?;
    // New options can change what a fetch returns, so fetch again.
    for provider in settings.shown() {
        refresh_provider(&app, provider);
    }
    Ok(settings)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            let settings = Settings::load(&settings_path);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(settings.pinned);
            }
            app.manage(AppState {
                settings_path,
                settings: Mutex::new(settings),
                in_flight: Mutex::new(HashSet::new()),
                timer_generations: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_meta,
            get_settings,
            start,
            refresh,
            refresh_all,
            set_auto_refresh,
            set_order,
            set_shown,
            set_pinned,
            save_preferences,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
