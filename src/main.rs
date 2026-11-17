use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{Local, TimeZone};
use clap::{Args, Parser, Subcommand};

use agent_usage::config::CODEX_EFFORTS;
use agent_usage::{Config, Metric, Provider, UsageReport};

#[derive(Parser)]
#[command(
    name = "agent-usage",
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

    /// Config file to use instead of the default location
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Codex (ChatGPT subscription) usage
    Codex(CodexArgs),
    /// Claude Code (Claude subscription) usage
    Claude(ClaudeArgs),
    /// Usage for every provider enabled in the config
    All,
}

#[derive(Args)]
struct CodexArgs {
    /// Model to probe [default: gpt-6-luna, or codex.model from the config]
    #[arg(long)]
    model: Option<String>,

    /// Reasoning effort for the probe [default: low, or codex.effort from the config]
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

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut config = match Config::load(cli.config.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };

    let (reports, single) = match cli.command {
        Command::Codex(args) => {
            if let Some(model) = args.model {
                config.codex.model = model;
            }
            if let Some(effort) = args.effort {
                config.codex.effort = effort;
            }
            if let Some(auth_file) = args.auth_file {
                config.codex.auth_file = Some(auth_file);
            }
            (vec![agent_usage::fetch(Provider::Codex, &config)], true)
        }
        Command::Claude(args) => {
            if let Some(credentials_file) = args.credentials_file {
                config.claude.credentials_file = Some(credentials_file);
            }
            if args.raw {
                return match agent_usage::claude::fetch_raw(&config.claude) {
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
            (vec![agent_usage::fetch(Provider::Claude, &config)], true)
        }
        Command::All => (agent_usage::fetch_all(&config), false),
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
