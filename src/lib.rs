//! Query subscription usage for coding agents.
//!
//! Each provider returns a [`UsageReport`] in one normalized shape, so a
//! caller (the `agent-usage` CLI, or a GUI) can treat them alike.

pub mod claude;
pub mod codex;
pub mod config;
pub mod model;

pub use config::Config;
pub use model::{Error, Metric, Provider, UsageReport};

/// Query one provider. Never fails: a failed query yields a report with
/// `error` set.
pub fn fetch(provider: Provider, config: &Config) -> UsageReport {
    let result = match provider {
        Provider::Codex => codex::fetch(&config.codex),
        Provider::Claude => claude::fetch(&config.claude),
    };
    result.unwrap_or_else(|err| UsageReport::failed(provider, err))
}

/// Query every provider enabled in the config, concurrently, returning
/// reports in the configured order.
pub fn fetch_all(config: &Config) -> Vec<UsageReport> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = config
            .providers
            .iter()
            .map(|&provider| (provider, scope.spawn(move || fetch(provider, config))))
            .collect();
        handles
            .into_iter()
            .map(|(provider, handle)| {
                handle.join().unwrap_or_else(|_| {
                    UsageReport::failed(provider, "The query crashed unexpectedly.")
                })
            })
            .collect()
    })
}
