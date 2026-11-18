// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let maintenance = match std::env::args().nth(1).as_deref() {
        Some("--unregister-cli") => {
            Some(agent_usage_gui_lib::cli_integration::set_enabled(false, true).map(|_| ()))
        }
        Some("--restore-cli") => Some(agent_usage_gui_lib::cli_integration::restore()),
        _ => None,
    };
    if let Some(result) = maintenance {
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    agent_usage_gui_lib::run()
}
