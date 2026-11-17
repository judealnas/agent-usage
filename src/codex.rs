//! Codex (ChatGPT subscription) usage.
//!
//! There is no usage endpoint, so this sends one minimal inference request
//! to the internal Codex endpoint and reads the quota from the `x-codex-*`
//! response headers. Each call consumes a very small amount of quota. The
//! endpoint is undocumented and may change without notice.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::CodexConfig;
use crate::model::{Error, Metric, Provider, UsageReport, FIVE_HOUR_MINUTES, WEEKLY_MINUTES};

const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const TOKEN_ENV: &str = "CODEX_ACCESS_TOKEN";
const ACCOUNT_ENV: &str = "CODEX_ACCOUNT_ID";

struct Credentials {
    access_token: String,
    account_id: Option<String>,
}

/// `auth.json` under `$CODEX_HOME` if set, else `~/.codex`.
fn default_auth_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|home| !home.is_empty()) {
        Some(home) => Some(PathBuf::from(home).join("auth.json")),
        None => dirs::home_dir().map(|home| home.join(".codex").join("auth.json")),
    }
}

/// Find credentials, in order of precedence:
///   1. `auth_file` (the `--auth-file` flag or `codex.auth_file` config)
///   2. `CODEX_ACCESS_TOKEN` (+ optional `CODEX_ACCOUNT_ID`)
///   3. `$CODEX_HOME/auth.json`
///   4. `~/.codex/auth.json`
fn load_credentials(auth_file: Option<&Path>) -> Result<Credentials, Error> {
    if auth_file.is_none() {
        if let Some(token) = std::env::var(TOKEN_ENV).ok().filter(|t| !t.is_empty()) {
            return Ok(Credentials {
                access_token: token,
                account_id: std::env::var(ACCOUNT_ENV).ok().filter(|a| !a.is_empty()),
            });
        }
    }

    let path = match auth_file {
        Some(path) => path.to_path_buf(),
        None => default_auth_path().ok_or_else(|| {
            Error::Credentials("Could not determine the home directory.".to_string())
        })?,
    };

    if !path.is_file() {
        return Err(Error::Credentials(format!(
            "Codex credentials not found: {}. Log in with the Codex CLI, or provide \
             credentials via --auth-file, {TOKEN_ENV}, or CODEX_HOME. (If the Codex CLI \
             stores credentials in the OS keyring rather than a file, set {TOKEN_ENV}.)",
            path.display()
        )));
    }

    let unreadable = |reason: String| {
        Error::Credentials(format!(
            "Could not read OAuth tokens from {}: {reason}",
            path.display()
        ))
    };

    let text = std::fs::read_to_string(&path).map_err(|err| unreadable(err.to_string()))?;
    let value: Value = serde_json::from_str(&text).map_err(|err| unreadable(err.to_string()))?;
    let tokens = &value["tokens"];
    let access_token = tokens["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| unreadable("no tokens.access_token".to_string()))?;

    Ok(Credentials {
        access_token: access_token.to_string(),
        account_id: tokens["account_id"].as_str().map(str::to_string),
    })
}

/// Extract one quota window (`primary` or `secondary`) from the headers.
/// Header names must be lowercase.
fn parse_window(name: &str, headers: &HashMap<String, String>) -> Option<Metric> {
    let get = |suffix: &str| {
        headers
            .get(&format!("x-codex-{name}-{suffix}"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };

    let used_percent: f64 = get("used-percent")?.parse().ok()?;
    let duration_minutes: Option<u32> = match get("window-minutes") {
        Some(value) => Some(value.parse().ok()?),
        None => None,
    };
    let reset_at: Option<i64> = match get("reset-at") {
        Some(value) => Some(value.parse().ok()?),
        None => None,
    };
    let reset_after: Option<i64> = match get("reset-after-seconds") {
        Some(value) => Some(value.parse().ok()?),
        None => None,
    };

    // Prefer the absolute reset time; fall back to the relative one.
    let resets_at =
        reset_at.or_else(|| reset_after.map(|secs| chrono::Utc::now().timestamp() + secs));

    // Identify windows by duration, not by primary/secondary.
    let label = match duration_minutes {
        Some(FIVE_HOUR_MINUTES) => "Five-hour".to_string(),
        Some(WEEKLY_MINUTES) => "Weekly".to_string(),
        Some(minutes) => format!("{minutes}-minute"),
        None => {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    };

    Some(Metric::window(
        label,
        duration_minutes,
        used_percent,
        resets_at,
    ))
}

/// Build a report from the HTTP status, the lowercase `x-codex-*` headers,
/// and the error body (for statuses of 400 and above).
fn build_report(
    status: u16,
    headers: &HashMap<String, String>,
    error_body: Option<&str>,
    config: &CodexConfig,
) -> UsageReport {
    let mut report = UsageReport::new(Provider::Codex);
    report.plan = headers.get("x-codex-plan-type").cloned();
    report.metrics = ["primary", "secondary"]
        .iter()
        .filter_map(|name| parse_window(name, headers))
        .collect();
    report.limit_reached = status == 429;

    report
        .details
        .insert("model".to_string(), config.model.clone());
    report
        .details
        .insert("effort".to_string(), config.effort.clone());
    let extras = [
        ("active_limit", "x-codex-active-limit"),
        ("rate_limit_reached_type", "x-codex-rate-limit-reached-type"),
    ];
    for (key, header) in extras {
        if let Some(value) = headers.get(header) {
            report.details.insert(key.to_string(), value.clone());
        }
    }

    // A 429 still carries the quota headers, so it is a successful read.
    let usable = (status == 200 || status == 429) && !report.metrics.is_empty();
    if !usable {
        report.error = Some(if status == 401 || status == 403 {
            format!(
                "Codex rejected the credentials (HTTP {status}). Run the Codex CLI to \
                 refresh the login."
            )
        } else if status == 200 || status == 429 {
            format!("Codex returned no quota headers (HTTP {status}).")
        } else {
            match error_body.filter(|body| !body.trim().is_empty()) {
                Some(body) => format!("Codex returned HTTP {status}: {}", body.trim()),
                None => format!("Codex returned HTTP {status}."),
            }
        });
    }

    report
}

/// Query Codex usage. Makes one real, minimal inference request.
pub fn fetch(config: &CodexConfig) -> Result<UsageReport, Error> {
    let credentials = load_credentials(config.auth_file.as_deref())?;

    // List-based input is required by the Codex endpoint.
    let payload = json!({
        "model": config.model,
        "instructions": "Reply with OK.",
        "input": [{
            "role": "user",
            "content": [{ "type": "input_text", "text": "OK" }],
        }],
        "stream": true,
        "store": false,
        "reasoning": { "effort": config.effort },
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build();
    let mut request = agent
        .post(ENDPOINT)
        .set(
            "Authorization",
            &format!("Bearer {}", credentials.access_token),
        )
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .set("originator", "codex_cli_rs");
    if let Some(account_id) = &credentials.account_id {
        request = request.set("ChatGPT-Account-Id", account_id);
    }

    let response = match request.send_string(&payload.to_string()) {
        Ok(response) => response,
        // Error statuses (including 429) still carry the quota headers.
        Err(ureq::Error::Status(_, response)) => response,
        Err(err) => return Err(Error::Http(format!("Codex request failed: {err}"))),
    };

    let status = response.status();
    let headers: HashMap<String, String> = response
        .headers_names()
        .into_iter()
        .map(|name| name.to_lowercase())
        .filter(|name| name.starts_with("x-codex-"))
        .filter_map(|name| {
            let value = response.header(&name)?.to_string();
            Some((name, value))
        })
        .collect();

    let mut error_body = None;
    let mut reader = response.into_reader();
    if status >= 400 {
        let mut raw = Vec::new();
        let _ = reader.take(2048).read_to_end(&mut raw);
        error_body = Some(String::from_utf8_lossy(&raw).into_owned());
    } else {
        // Drain the stream so the request completes normally. The generated
        // text is discarded.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    }

    Ok(build_report(
        status,
        &headers,
        error_body.as_deref(),
        config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sample_headers() -> HashMap<String, String> {
        headers(&[
            ("x-codex-primary-used-percent", "5"),
            ("x-codex-primary-window-minutes", "300"),
            ("x-codex-primary-reset-at", "1794885647"),
            ("x-codex-primary-reset-after-seconds", "1372"),
            ("x-codex-secondary-used-percent", "1.5"),
            ("x-codex-secondary-window-minutes", "10080"),
            ("x-codex-secondary-reset-at", "1795479528"),
            ("x-codex-secondary-reset-after-seconds", "595253"),
            ("x-codex-active-limit", "premium"),
            ("x-codex-plan-type", "edu_plus"),
        ])
    }

    #[test]
    fn parses_both_windows() {
        let report = build_report(200, &sample_headers(), None, &CodexConfig::default());
        assert!(report.is_ok());
        assert!(!report.limit_reached);
        assert_eq!(report.plan.as_deref(), Some("edu_plus"));
        assert_eq!(
            report.metrics,
            vec![
                Metric::Window {
                    label: "Five-hour".to_string(),
                    duration_minutes: Some(300),
                    used_percent: 5.0,
                    remaining_percent: 95.0,
                    resets_at: Some(1794885647),
                },
                Metric::Window {
                    label: "Weekly".to_string(),
                    duration_minutes: Some(10080),
                    used_percent: 1.5,
                    remaining_percent: 98.5,
                    resets_at: Some(1795479528),
                },
            ]
        );
        assert_eq!(report.details["active_limit"], "premium");
        assert_eq!(report.details["model"], "gpt-6-luna");
    }

    #[test]
    fn rate_limited_response_is_still_a_reading() {
        let mut h = sample_headers();
        h.insert(
            "x-codex-primary-used-percent".to_string(),
            "100".to_string(),
        );
        h.insert(
            "x-codex-rate-limit-reached-type".to_string(),
            "primary".to_string(),
        );
        let report = build_report(429, &h, Some("{}"), &CodexConfig::default());
        assert!(report.is_ok());
        assert!(report.limit_reached);
        assert_eq!(report.details["rate_limit_reached_type"], "primary");
        match &report.metrics[0] {
            Metric::Window {
                remaining_percent, ..
            } => assert_eq!(*remaining_percent, 0.0),
            other => panic!("unexpected metric {other:?}"),
        }
    }

    #[test]
    fn unusual_window_length_gets_a_generic_label() {
        let h = headers(&[
            ("x-codex-primary-used-percent", "12"),
            ("x-codex-primary-window-minutes", "60"),
        ]);
        let report = build_report(200, &h, None, &CodexConfig::default());
        match &report.metrics[0] {
            Metric::Window {
                label, resets_at, ..
            } => {
                assert_eq!(label, "60-minute");
                assert_eq!(*resets_at, None);
            }
            other => panic!("unexpected metric {other:?}"),
        }
    }

    #[test]
    fn malformed_window_is_dropped() {
        let h = headers(&[
            ("x-codex-primary-used-percent", "lots"),
            ("x-codex-secondary-used-percent", "3"),
            ("x-codex-secondary-window-minutes", "10080"),
        ]);
        let report = build_report(200, &h, None, &CodexConfig::default());
        assert_eq!(report.metrics.len(), 1);
    }

    #[test]
    fn missing_headers_is_an_error() {
        let report = build_report(200, &HashMap::new(), None, &CodexConfig::default());
        assert!(report.error.unwrap().contains("no quota headers"));
    }

    #[test]
    fn unauthorized_points_at_the_codex_cli() {
        let report = build_report(401, &HashMap::new(), Some("{}"), &CodexConfig::default());
        assert!(report.error.unwrap().contains("Run the Codex CLI"));
    }

    #[test]
    fn server_error_includes_the_body() {
        let report = build_report(
            400,
            &HashMap::new(),
            Some(r#"{"detail":"Unsupported model"}"#),
            &CodexConfig::default(),
        );
        let error = report.error.unwrap();
        assert!(error.contains("HTTP 400"));
        assert!(error.contains("Unsupported model"));
    }
}
