//! Query subscription usage for coding agents.
//!
//! Each provider returns a [`UsageReport`] in one normalized shape, so a
//! caller (the `agent-usage-cli` CLI, or a GUI) can treat them alike.

pub mod claude;
pub mod codex;
pub mod codex_account;
pub mod model;
pub mod options;
pub mod sources;

pub use model::{Error, Metric, Provider, UsageReport};
pub use options::{ClaudeOptions, CodexOptions, Options};

/// Query one provider. Never fails: a failed query yields a report with
/// `error` set.
pub fn fetch(provider: Provider, options: &Options) -> UsageReport {
    let result = match provider {
        Provider::Codex => codex::fetch(&options.codex),
        Provider::Claude => claude::fetch(&options.claude),
    };
    result.unwrap_or_else(|err| UsageReport::failed(provider, err))
}

/// Query the given providers concurrently, returning reports in the same
/// order.
pub fn fetch_all(providers: &[Provider], options: &Options) -> Vec<UsageReport> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = providers
            .iter()
            .map(|&provider| (provider, scope.spawn(move || fetch(provider, options))))
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
