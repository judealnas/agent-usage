//! Claude Code (Claude subscription) usage.
//!
//! Reads the OAuth token Claude Code stores on disk and asks Anthropic's
//! usage endpoint for the current rate-limit windows. The endpoint is
//! undocumented and its schema changes, so parsing is defensive: anything
//! missing or oddly shaped is skipped, not treated as fatal.
//!
//! The token is never refreshed here. Claude Code refreshes the file
//! itself, so the file is re-read on every call.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::model::{Error, Metric, Provider, UsageReport, FIVE_HOUR_MINUTES, WEEKLY_MINUTES};
use crate::options::ClaudeOptions;

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const EXPIRED: &str = "Claude login has expired. Open Claude Code to refresh it.";

struct Credentials {
    access_token: String,
    /// Expiry as Unix epoch milliseconds.
    expires_at_ms: Option<i64>,
    plan: Option<String>,
}

/// `.credentials.json` under `$CLAUDE_CONFIG_DIR` if set, else `~/.claude`.
fn default_credentials_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join(".credentials.json")),
        None => dirs::home_dir().map(|home| home.join(".claude").join(".credentials.json")),
    }
}

fn parse_credentials(text: &str) -> Result<Credentials, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let oauth = &value["claudeAiOauth"];
    let access_token = oauth["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or("no claudeAiOauth.accessToken")?;

    Ok(Credentials {
        access_token: access_token.to_string(),
        expires_at_ms: oauth["expiresAt"].as_i64(),
        plan: oauth["subscriptionType"].as_str().map(str::to_string),
    })
}

fn load_credentials(credentials_file: Option<&Path>) -> Result<Credentials, Error> {
    let path = match credentials_file {
        Some(path) => path.to_path_buf(),
        None => default_credentials_path().ok_or_else(|| {
            Error::Credentials("Could not determine the home directory.".to_string())
        })?,
    };

    if !path.is_file() {
        return Err(Error::Credentials(format!(
            "Claude credentials not found: {}. Log in with Claude Code, or give the path to its .credentials.json.",
            path.display()
        )));
    }

    let unreadable = |reason: String| {
        Error::Credentials(format!(
            "Could not read the OAuth token from {}: {reason}",
            path.display()
        ))
    };
    let text = std::fs::read_to_string(&path).map_err(|err| unreadable(err.to_string()))?;
    let credentials = parse_credentials(&text).map_err(unreadable)?;

    if let Some(expires_at_ms) = credentials.expires_at_ms {
        if expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            return Err(Error::Credentials(EXPIRED.to_string()));
        }
    }

    Ok(credentials)
}

/// Fetch the raw JSON body from the usage endpoint.
fn request(credentials: &Credentials) -> Result<String, Error> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let result = agent
        .get(ENDPOINT)
        .set(
            "Authorization",
            &format!("Bearer {}", credentials.access_token),
        )
        .set("anthropic-beta", OAUTH_BETA)
        .set("Accept", "application/json")
        .set(
            "User-Agent",
            concat!("agent-usage-cli/", env!("CARGO_PKG_VERSION")),
        )
        .call();

    match result {
        Ok(response) => response
            .into_string()
            .map_err(|err| Error::Http(format!("Could not read the Claude usage response: {err}"))),
        Err(ureq::Error::Status(401, _)) => Err(Error::Credentials(EXPIRED.to_string())),
        Err(ureq::Error::Status(429, _)) => Err(Error::Http(
            "The Claude usage endpoint is rate-limiting requests (HTTP 429). Try again later."
                .to_string(),
        )),
        Err(ureq::Error::Status(status, response)) => {
            let body: String = response
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            Err(Error::Http(if body.trim().is_empty() {
                format!("The Claude usage endpoint returned HTTP {status}.")
            } else {
                format!(
                    "The Claude usage endpoint returned HTTP {status}: {}",
                    body.trim()
                )
            }))
        }
        Err(err) => Err(Error::Http(format!("Claude usage request failed: {err}"))),
    }
}

/// Parse a reset time: an RFC 3339 string, or epoch seconds.
fn parse_reset(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.timestamp()),
        Value::Number(number) => number.as_f64().map(|secs| secs as i64),
        _ => None,
    }
}

/// Parse one `{utilization, resets_at}` object into a window.
fn parse_window(value: &Value, label: &str, duration_minutes: u32) -> Option<Metric> {
    let used_percent = value.get("utilization")?.as_f64()?;
    let resets_at = value.get("resets_at").and_then(parse_reset);
    Some(Metric::window(
        label,
        Some(duration_minutes),
        used_percent,
        resets_at,
    ))
}

/// Turn the usage endpoint's JSON body into metrics.
fn parse_usage(body: &Value) -> Vec<Metric> {
    let mut metrics = Vec::new();

    let windows = [
        ("five_hour", "Five-hour", FIVE_HOUR_MINUTES),
        ("seven_day", "Weekly", WEEKLY_MINUTES),
        ("seven_day_opus", "Weekly (Opus)", WEEKLY_MINUTES),
        ("seven_day_sonnet", "Weekly (Sonnet)", WEEKLY_MINUTES),
    ];
    for (key, label, minutes) in windows {
        if let Some(metric) = body
            .get(key)
            .and_then(|value| parse_window(value, label, minutes))
        {
            metrics.push(metric);
        }
    }

    // The newer `limits` array repeats the session and weekly windows and
    // adds model-scoped weekly limits. Take from it only what is missing.
    let limits = body
        .get("limits")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for limit in limits {
        let Some(used_percent) = limit.get("percent").and_then(Value::as_f64) else {
            continue;
        };
        let (label, minutes) = match limit.get("kind").and_then(Value::as_str) {
            Some("session") => ("Five-hour".to_string(), FIVE_HOUR_MINUTES),
            Some("weekly_all") => ("Weekly".to_string(), WEEKLY_MINUTES),
            Some("weekly_scoped") => {
                let Some(model) = limit
                    .pointer("/scope/model/display_name")
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                (format!("Weekly ({model})"), WEEKLY_MINUTES)
            }
            _ => continue,
        };
        let already_present = metrics.iter().any(
            |metric| matches!(metric, Metric::Window { label: existing, .. } if *existing == label),
        );
        if !already_present {
            let resets_at = limit.get("resets_at").and_then(parse_reset);
            metrics.push(Metric::window(
                label,
                Some(minutes),
                used_percent,
                resets_at,
            ));
        }
    }

    if let Some(balance) = body.get("extra_usage").and_then(parse_extra_usage) {
        metrics.push(balance);
    }

    metrics
}

/// Parse pay-as-you-go extra usage, when it is switched on. Amounts arrive
/// in minor units (cents for USD) alongside a `decimal_places` field.
fn parse_extra_usage(value: &Value) -> Option<Metric> {
    if value.get("is_enabled")?.as_bool() != Some(true) {
        return None;
    }
    let decimal_places = value
        .get("decimal_places")
        .and_then(Value::as_i64)
        .unwrap_or(2);
    let scale = 10f64.powi(decimal_places.clamp(0, 6) as i32);
    let amount = |key: &str| value.get(key).and_then(Value::as_f64).map(|v| v / scale);

    let used = amount("used_credits");
    let limit = amount("monthly_limit");
    if used.is_none() && limit.is_none() {
        return None;
    }
    Some(Metric::Balance {
        label: "Extra usage".to_string(),
        used,
        limit,
        remaining: match (used, limit) {
            (Some(used), Some(limit)) => Some((limit - used).max(0.0)),
            _ => None,
        },
        currency: value
            .get("currency")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Fetch the raw response body, for inspecting schema changes.
pub fn fetch_raw(config: &ClaudeOptions) -> Result<String, Error> {
    let credentials = load_credentials(config.credentials_file.as_deref())?;
    request(&credentials)
}

/// Query Claude Code usage. Costs no quota.
pub fn fetch(config: &ClaudeOptions) -> Result<UsageReport, Error> {
    let credentials = load_credentials(config.credentials_file.as_deref())?;
    let text = request(&credentials)?;
    let body: Value = serde_json::from_str(&text).map_err(|err| {
        Error::Parse(format!(
            "The Claude usage response was not valid JSON: {err}"
        ))
    })?;

    let mut report = UsageReport::new(Provider::Claude);
    report.plan = credentials.plan;
    report.metrics = parse_usage(&body);
    if report.metrics.is_empty() {
        return Err(Error::Parse(
            "The Claude usage response contained no recognizable usage windows. The \
             endpoint's format may have changed; run `agent-usage-cli claude --raw` to inspect it."
                .to_string(),
        ));
    }
    report.limit_reached = report.metrics.iter().any(
        |metric| matches!(metric, Metric::Window { used_percent, .. } if *used_percent >= 100.0),
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn window(label: &str, minutes: u32, used: f64, resets_at: Option<i64>) -> Metric {
        Metric::window(label, Some(minutes), used, resets_at)
    }

    #[test]
    fn parses_a_real_response() {
        let body: Value =
            serde_json::from_str(include_str!("../tests/fixtures/claude_usage.json")).unwrap();
        assert_eq!(
            parse_usage(&body),
            vec![
                window("Five-hour", 300, 42.0, Some(1794890999)),
                window("Weekly", 10080, 31.5, Some(1795147199)),
                window("Weekly (Fable)", 10080, 12.0, Some(1795147199)),
            ]
        );
    }

    #[test]
    fn legacy_model_windows_are_not_duplicated_by_limits() {
        let body = json!({
            "seven_day_opus": { "utilization": 9.0, "resets_at": null },
            "limits": [{
                "kind": "weekly_scoped",
                "percent": 9,
                "scope": { "model": { "display_name": "Opus" } },
            }],
        });
        assert_eq!(
            parse_usage(&body),
            vec![window("Weekly (Opus)", 10080, 9.0, None)]
        );
    }

    #[test]
    fn falls_back_to_limits_when_top_level_windows_are_missing() {
        let body = json!({
            "five_hour": null,
            "limits": [
                { "kind": "session", "percent": 7, "resets_at": 1794890999 },
                { "kind": "weekly_all", "percent": 3 },
                { "kind": "something_new", "percent": 50 },
                { "kind": "weekly_scoped", "percent": 1, "scope": null },
            ],
        });
        assert_eq!(
            parse_usage(&body),
            vec![
                window("Five-hour", 300, 7.0, Some(1794890999)),
                window("Weekly", 10080, 3.0, None),
            ]
        );
    }

    #[test]
    fn tolerates_missing_and_wrongly_typed_fields() {
        for body in [
            json!({}),
            json!([]),
            json!(null),
            json!({ "five_hour": "soon", "seven_day": { "utilization": "high" }, "limits": {} }),
            json!({ "five_hour": { "resets_at": "2026-11-17T04:49:59Z" }, "limits": [1, null] }),
        ] {
            assert_eq!(parse_usage(&body), vec![], "body: {body}");
        }
    }

    #[test]
    fn unparseable_reset_time_keeps_the_window() {
        let body = json!({ "five_hour": { "utilization": 10, "resets_at": "tomorrow" } });
        assert_eq!(
            parse_usage(&body),
            vec![window("Five-hour", 300, 10.0, None)]
        );
    }

    #[test]
    fn enabled_extra_usage_becomes_a_balance() {
        let body = json!({
            "extra_usage": {
                "is_enabled": true,
                "monthly_limit": 5000,
                "used_credits": 1250,
                "currency": "USD",
                "decimal_places": 2,
            },
        });
        assert_eq!(
            parse_usage(&body),
            vec![Metric::Balance {
                label: "Extra usage".to_string(),
                used: Some(12.5),
                limit: Some(50.0),
                remaining: Some(37.5),
                currency: Some("USD".to_string()),
            }]
        );
    }

    #[test]
    fn reads_the_token_and_plan_from_credentials() {
        let credentials = parse_credentials(
            r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":1794890999000,"subscriptionType":"team"}}"#,
        )
        .unwrap();
        assert_eq!(credentials.access_token, "tok");
        assert_eq!(credentials.expires_at_ms, Some(1794890999000));
        assert_eq!(credentials.plan.as_deref(), Some("team"));
    }

    #[test]
    fn credentials_without_a_token_are_rejected() {
        assert!(parse_credentials(r#"{"claudeAiOauth":{}}"#).is_err());
        assert!(parse_credentials(r#"{"mcpOAuth":{}}"#).is_err());
        assert!(parse_credentials("not json").is_err());
    }
}
