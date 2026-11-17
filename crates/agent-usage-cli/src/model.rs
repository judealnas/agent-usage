//! The normalized usage shape shared by every provider.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const FIVE_HOUR_MINUTES: u32 = 300;
pub const WEEKLY_MINUTES: u32 = 10080;

/// A provider this crate knows how to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Codex, Provider::Claude];

    /// Lowercase identifier used on the command line and in JSON.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Codex => "codex",
            Provider::Claude => "claude",
        }
    }

    /// Human-readable name.
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Codex => "Codex",
            Provider::Claude => "Claude Code",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// One usage figure. Subscription providers report rate-limit windows;
/// credit-based providers report a balance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Metric {
    /// A rate-limit window: percent used, and when it resets.
    Window {
        label: String,
        /// Window length, when the provider states it.
        duration_minutes: Option<u32>,
        used_percent: f64,
        remaining_percent: f64,
        /// Reset time as Unix epoch seconds.
        resets_at: Option<i64>,
    },
    /// A credit balance in a currency.
    Balance {
        label: String,
        used: Option<f64>,
        limit: Option<f64>,
        remaining: Option<f64>,
        currency: Option<String>,
    },
}

impl Metric {
    pub fn window(
        label: impl Into<String>,
        duration_minutes: Option<u32>,
        used_percent: f64,
        resets_at: Option<i64>,
    ) -> Metric {
        Metric::Window {
            label: label.into(),
            duration_minutes,
            used_percent,
            remaining_percent: (100.0 - used_percent).max(0.0),
            resets_at,
        }
    }
}

/// The result of querying one provider. A failed query still produces a
/// report, with `error` set and `metrics` empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageReport {
    pub provider: Provider,
    pub plan: Option<String>,
    pub metrics: Vec<Metric>,
    /// True when the provider says a limit has been hit.
    pub limit_reached: bool,
    /// Provider-specific extras, such as the Codex model that was probed.
    pub details: BTreeMap<String, String>,
    /// When this report was produced, as Unix epoch seconds.
    pub fetched_at: i64,
    pub error: Option<String>,
}

impl UsageReport {
    pub fn new(provider: Provider) -> UsageReport {
        UsageReport {
            provider,
            plan: None,
            metrics: Vec::new(),
            limit_reached: false,
            details: BTreeMap::new(),
            fetched_at: chrono::Utc::now().timestamp(),
            error: None,
        }
    }

    pub fn failed(provider: Provider, error: impl fmt::Display) -> UsageReport {
        let mut report = UsageReport::new(provider);
        report.error = Some(error.to_string());
        report
    }

    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Why a provider query failed.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Credentials are missing, unreadable, or expired.
    Credentials(String),
    /// The request could not be completed, or the server refused it.
    Http(String),
    /// The response did not contain usable usage data.
    Parse(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Credentials(m) | Error::Http(m) | Error::Parse(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(text: &str) -> Result<Provider, String> {
        Provider::ALL
            .into_iter()
            .find(|provider| provider.id() == text.trim().to_lowercase())
            .ok_or_else(|| {
                let known: Vec<_> = Provider::ALL.iter().map(|p| p.id()).collect();
                format!("unknown provider \"{text}\" (known: {})", known.join(", "))
            })
    }
}
