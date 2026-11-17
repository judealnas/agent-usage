use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{Local, TimeZone};
use clap::{Args, Parser, Subcommand};

use agent_usage_cli::options::CODEX_EFFORTS;
use agent_usage_cli::{Metric, Options, Provider, UsageReport};

#[derive(Parser)]
#[command(
    name = "agent-usage-cli",
    version,
    about = "Show subscription usage for coding agents",
    after_help = "Both providers are queried through undocumented endpoints using your \
                  existing login tokens. The Codex query sends one minimal inference \
                  request, which consumes a very small amount of quota."
)]
struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Codex (ChatGPT subscription) usage
    Codex(CodexArgs),
    /// Claude Code (Claude subscription) usage
    Claude(ClaudeArgs),
    /// Usage for every provider
    All(AllArgs),
}

#[derive(Args)]
struct CodexArgs {
    /// Model to probe [default: gpt-6-luna]
    #[arg(long)]
    model: Option<String>,

    /// Reasoning effort for the probe [default: low]
    #[arg(long, value_parser = CODEX_EFFORTS)]
    effort: Option<String>,

    /// Path to a Codex auth.json. Overrides CODEX_ACCESS_TOKEN and CODEX_HOME
    #[arg(long, value_name = "PATH")]
    auth_file: Option<PathBuf>,
}

#[derive(Args)]
struct ClaudeArgs {
    /// Path to Claude Code's .credentials.json
    #[arg(long, value_name = "PATH")]
    credentials_file: Option<PathBuf>,

    /// Print the usage endpoint's raw response and exit
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct AllArgs {
    /// Providers to query, in order, comma-separated [default: every provider]
    #[arg(long, value_name = "PROVIDERS", value_delimiter = ',')]
    only: Option<Vec<Provider>>,

    /// Codex model to probe [default: gpt-6-luna]
    #[arg(long, value_name = "MODEL")]
    codex_model: Option<String>,

    /// Codex reasoning effort for the probe [default: low]
    #[arg(long, value_name = "EFFORT", value_parser = CODEX_EFFORTS)]
    codex_effort: Option<String>,

    /// Path to a Codex auth.json
    #[arg(long, value_name = "PATH")]
    codex_auth_file: Option<PathBuf>,

    /// Path to Claude Code's .credentials.json
    #[arg(long, value_name = "PATH")]
    claude_credentials_file: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut options = Options::default();

    let (reports, single) = match cli.command {
        Command::Codex(args) => {
            if let Some(model) = args.model {
                options.codex.model = model;
            }
            if let Some(effort) = args.effort {
                options.codex.effort = effort;
            }
            options.codex.auth_file = args.auth_file;
            (
                vec![agent_usage_cli::fetch(Provider::Codex, &options)],
                true,
            )
        }
        Command::Claude(args) => {
            options.claude.credentials_file = args.credentials_file;
            if args.raw {
                return match agent_usage_cli::claude::fetch_raw(&options.claude) {
                    Ok(body) => {
                        println!("{body}");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        eprintln!("{err}");
                        ExitCode::FAILURE
                    }
                };
            }
            (
                vec![agent_usage_cli::fetch(Provider::Claude, &options)],
                true,
            )
        }
        Command::All(args) => {
            if let Some(model) = args.codex_model {
                options.codex.model = model;
            }
            if let Some(effort) = args.codex_effort {
                options.codex.effort = effort;
            }
            options.codex.auth_file = args.codex_auth_file;
            options.claude.credentials_file = args.claude_credentials_file;
            let providers = args.only.unwrap_or_else(|| Provider::ALL.to_vec());
            (agent_usage_cli::fetch_all(&providers, &options), false)
        }
    };

    if cli.json {
        // One provider prints an object; `all` prints an array.
        let output = if single {
            serde_json::to_string_pretty(&reports[0])
        } else {
            serde_json::to_string_pretty(&reports)
        };
        println!("{}", output.expect("reports serialize to JSON"));
    } else {
        let now = chrono::Utc::now().timestamp();
        for (index, report) in reports.iter().enumerate() {
            if index > 0 {
                println!();
            }
            print_report(report, now);
        }
    }

    if reports.iter().all(UsageReport::is_ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_report(report: &UsageReport, now: i64) {
    match &report.plan {
        Some(plan) => println!("{} ({plan})", report.provider.display_name()),
        None => println!("{}", report.provider.display_name()),
    }

    if let Some(error) = &report.error {
        println!("  Error: {error}");
        return;
    }

    let label_width = report
        .metrics
        .iter()
        .map(|metric| match metric {
            Metric::Window { label, .. } | Metric::Balance { label, .. } => label.chars().count(),
        })
        .max()
        .unwrap_or(0);

    for metric in &report.metrics {
        match metric {
            Metric::Window {
                label,
                used_percent,
                resets_at,
                ..
            } => {
                let used = format!("{}% used", format_number(*used_percent));
                match resets_at {
                    Some(resets_at) => println!(
                        "  {label:<label_width$}  {used:<11}  resets in {} ({})",
                        format_countdown(resets_at - now),
                        format_local(*resets_at, now),
                    ),
                    None => println!("  {label:<label_width$}  {used}"),
                }
            }
            Metric::Balance {
                label,
                used,
                limit,
                remaining,
                currency,
            } => {
                let unit = currency.as_deref().unwrap_or("");
                let mut parts = Vec::new();
                match (used, limit) {
                    (Some(used), Some(limit)) => parts.push(format!(
                        "{} of {} {unit} used",
                        format_number(*used),
                        format_number(*limit)
                    )),
                    (Some(used), None) => {
                        parts.push(format!("{} {unit} used", format_number(*used)))
                    }
                    _ => {}
                }
                if let Some(remaining) = remaining {
                    parts.push(format!("{} {unit} remaining", format_number(*remaining)));
                }
                println!("  {label:<label_width$}  {}", parts.join(", "));
            }
        }
    }

    if report.limit_reached {
        println!("  Limit reached.");
    }
}

/// Up to two decimals, without trailing zeros: 5, 1.5, 12.25.
fn format_number(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_countdown(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// Local reset time: time only when it is today, otherwise with the day.
fn format_local(timestamp: i64, now: i64) -> String {
    let (Some(time), Some(today)) = (
        Local.timestamp_opt(timestamp, 0).single(),
        Local.timestamp_opt(now, 0).single(),
    ) else {
        return "unknown time".to_string();
    };
    if time.date_naive() == today.date_naive() {
        time.format("%-I:%M %p").to_string()
    } else {
        time.format("%a %b %-d, %-I:%M %p").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(args: &[&str]) -> Result<Option<Vec<Provider>>, clap::Error> {
        let cli = Cli::try_parse_from(args)?;
        match cli.command {
            Command::All(all) => Ok(all.only),
            _ => panic!("expected the all subcommand"),
        }
    }

    #[test]
    fn only_keeps_the_given_order() {
        assert_eq!(
            only(&["agent-usage-cli", "all", "--only", "claude,codex"]).unwrap(),
            Some(vec![Provider::Claude, Provider::Codex])
        );
        assert_eq!(only(&["agent-usage-cli", "all"]).unwrap(), None);
    }

    #[test]
    fn only_rejects_unknown_providers() {
        assert!(only(&["agent-usage-cli", "all", "--only", "codex,nope"]).is_err());
    }

    #[test]
    fn config_flag_no_longer_exists() {
        assert!(Cli::try_parse_from(["agent-usage-cli", "all", "--config", "x"]).is_err());
    }

    #[test]
    fn invalid_effort_is_rejected() {
        assert!(Cli::try_parse_from(["agent-usage-cli", "codex", "--effort", "extreme"]).is_err());
    }
}
