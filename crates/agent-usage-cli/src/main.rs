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
    after_help = "Codex defaults to an inference probe that consumes quota. Use --source account \
                  for the official Codex App Server login and a read-only quota query. \
                  Inference and Claude queries use undocumented HTTP endpoints."
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
    /// Usage measurement strategy (account uses the official Codex login)
    #[arg(long, default_value = "inference", value_parser = ["inference", "account", "both"])]
    source: String,
    /// Installed Codex executable for account queries
    #[arg(long)]
    codex_bin: Option<PathBuf>,
    /// Codex home directory for account queries (separate from --auth-file)
    #[arg(long)]
    codex_home: Option<PathBuf>,
    /// Model to probe [default: gpt-6-luna]
    #[arg(long)]
    model: Option<String>,

    /// Reasoning effort for the probe [default: low]
    #[arg(long, value_parser = CODEX_EFFORTS)]
    effort: Option<String>,

    /// Path to a Codex auth.json. Overrides CODEX_ACCESS_TOKEN and CODEX_HOME
    #[arg(long, value_name = "PATH")]
    auth_file: Option<PathBuf>,

    /// Print the HTTP status and raw quota headers as JSON and exit
    #[arg(long)]
    raw: bool,
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
    #[arg(long, default_value = "inference", value_parser = ["inference", "account", "both"])]
    codex_source: String,
    #[arg(long)]
    codex_bin: Option<PathBuf>,
    #[arg(long)]
    codex_home: Option<PathBuf>,
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
            if let Err(error) = validate_source_options(
                &args.source,
                args.model.is_some() || args.effort.is_some() || args.auth_file.is_some(),
                args.codex_bin.is_some() || args.codex_home.is_some(),
            ) {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
            let account = account_options(args.codex_bin, args.codex_home);
            if let Some(model) = args.model {
                options.codex.model = model;
            }
            if let Some(effort) = args.effort {
                options.codex.effort = effort;
            }
            options.codex.auth_file = args.auth_file;
            if args.source != "inference" {
                return print_sources(&args.source, &options, &account, args.raw, cli.json, &[]);
            }
            if args.raw {
                return match agent_usage_cli::codex::fetch_raw(&options.codex) {
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
            if let Err(error) = validate_source_options(
                &args.codex_source,
                args.codex_model.is_some()
                    || args.codex_effort.is_some()
                    || args.codex_auth_file.is_some(),
                args.codex_bin.is_some() || args.codex_home.is_some(),
            ) {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
            let account = account_options(args.codex_bin, args.codex_home);
            if let Some(model) = args.codex_model {
                options.codex.model = model;
            }
            if let Some(effort) = args.codex_effort {
                options.codex.effort = effort;
            }
            options.codex.auth_file = args.codex_auth_file;
            options.claude.credentials_file = args.claude_credentials_file;
            let providers = args.only.unwrap_or_else(|| Provider::ALL.to_vec());
            if args.codex_source != "inference" && providers.contains(&Provider::Codex) {
                let others: Vec<_> = providers
                    .iter()
                    .copied()
                    .filter(|p| *p != Provider::Codex)
                    .collect();
                return print_sources(
                    &args.codex_source,
                    &options,
                    &account,
                    false,
                    cli.json,
                    &others,
                );
            }
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

fn validate_source_options(
    source: &str,
    inference: bool,
    account: bool,
) -> Result<(), &'static str> {
    if source == "account" && inference {
        return Err("Model, effort, and auth-file options require inference or both.");
    }
    if source == "inference" && account {
        return Err("Codex-bin and codex-home require account or both.");
    }
    Ok(())
}

fn account_options(
    executable: Option<PathBuf>,
    codex_home: Option<PathBuf>,
) -> agent_usage_cli::codex_account::AccountOptions {
    let mut result = agent_usage_cli::codex_account::AccountOptions::default();
    if let Some(path) = executable {
        result.executable = path;
    }
    result.codex_home = codex_home;
    result
}

fn print_sources(
    selection: &str,
    options: &Options,
    account: &agent_usage_cli::codex_account::AccountOptions,
    raw: bool,
    json_output: bool,
    others: &[Provider],
) -> ExitCode {
    use agent_usage_cli::sources::{self, CodexSource};
    let selected = if selection == "both" {
        vec![CodexSource::Account, CodexSource::Inference]
    } else {
        vec![CodexSource::Account]
    };
    if raw {
        let results: Vec<_> = std::thread::scope(|scope| {
            let jobs: Vec<_> = selected
                .iter()
                .map(|&source| {
                    scope.spawn(move || (source, sources::raw(source, &options.codex, account)))
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join().expect("raw query worker"))
                .collect()
        });
        let ok = results.iter().all(|(_, r)| r.is_ok());
        let output = if results.len() == 1 && ok {
            results[0].1.as_ref().unwrap().clone()
        } else {
            serde_json::json!({"schema_version":2,"sources": results.into_iter().map(|(source,result)| match result {
                Ok(result) => serde_json::json!({"source":source,"result":result}),
                Err(error) => serde_json::json!({"source":source,"error":error}),
            }).collect::<Vec<_>>()})
        };
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let reports: Vec<_> = std::thread::scope(|scope| {
        let jobs: Vec<_> = selected
            .iter()
            .map(|&source| scope.spawn(move || sources::fetch(source, &options.codex, account)))
            .collect();
        jobs.into_iter()
            .map(|job| job.join().expect("source query worker"))
            .collect()
    });
    let other_reports = agent_usage_cli::fetch_all(others, options);
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"schema_version":2,"sources":reports,"providers":other_reports})
            )
            .unwrap()
        );
    } else {
        for report in &reports {
            println!("Codex — {:?}", report.source);
            if let Some(error) = &report.error {
                println!("  Error: {error}");
            }
            for group in &report.groups {
                println!(
                    "  {}",
                    group
                        .limit_name
                        .as_deref()
                        .or(group.limit_id.as_deref())
                        .unwrap_or(&group.key)
                );
                let mut legacy = UsageReport::new(Provider::Codex);
                legacy.plan = group.plan.clone();
                legacy.metrics = group.metrics.clone();
                legacy.limit_reached = group.rate_limit_reached_type.is_some()
                    || group.ordinary_usage_allowed == Some(false)
                    || group.spend_control_reached == Some(true);
                print_report(&legacy, chrono::Utc::now().timestamp());
                if let Some(error) = &group.error {
                    println!("  Error: {error}");
                }
            }
        }
        for report in &other_reports {
            print_report(report, chrono::Utc::now().timestamp());
        }
    }
    if reports.iter().all(|r| r.is_ok()) && other_reports.iter().all(UsageReport::is_ok) {
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

    #[test]
    fn codex_raw_accepts_query_options_and_json() {
        let cli = Cli::try_parse_from([
            "agent-usage-cli",
            "codex",
            "--raw",
            "--json",
            "--model",
            "example-model",
            "--effort",
            "low",
            "--auth-file",
            "example-auth.json",
        ])
        .unwrap();
        assert!(cli.json);
        let Command::Codex(args) = cli.command else {
            panic!("expected codex")
        };
        assert!(args.raw);
        assert_eq!(args.model.as_deref(), Some("example-model"));
        assert_eq!(args.effort.as_deref(), Some("low"));
        assert_eq!(args.auth_file, Some(PathBuf::from("example-auth.json")));
    }
}
