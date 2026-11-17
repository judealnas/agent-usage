//! Optional TOML configuration shared by the CLI and the GUI.
//!
//! Everything works without a config file; every field has a default.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{Error, Provider};

pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-luna";
pub const DEFAULT_CODEX_EFFORT: &str = "low";
pub const CODEX_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Providers queried by `agent-usage all` and shown in the GUI.
    pub providers: Vec<Provider>,
    /// How often the GUI polls. The CLI ignores this.
    pub poll_interval_seconds: u64,
    pub codex: CodexConfig,
    pub claude: ClaudeConfig,
    pub thresholds: Thresholds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexConfig {
    /// Model used for the probe request.
    pub model: String,
    /// Reasoning effort used for the probe request.
    pub effort: String,
    /// Path to a Codex `auth.json`. Overrides the environment lookups.
    pub auth_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Path to Claude Code's `.credentials.json`.
    pub credentials_file: Option<PathBuf>,
}

/// Percent-used levels at which the GUI changes a bar's color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub warn: f64,
    pub critical: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            providers: Provider::ALL.to_vec(),
            poll_interval_seconds: 300,
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            thresholds: Thresholds::default(),
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        CodexConfig {
            model: DEFAULT_CODEX_MODEL.to_string(),
            effort: DEFAULT_CODEX_EFFORT.to_string(),
            auth_file: None,
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            warn: 70.0,
            critical: 90.0,
        }
    }
}

impl Config {
    /// Where the config file lives by default:
    /// `%APPDATA%\agent-usage\config.toml` on Windows, and the platform's
    /// equivalent config directory elsewhere.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("agent-usage").join("config.toml"))
    }

    /// Load the config. With `path` given, the file must exist. Without
    /// it, the default location is used and a missing file means defaults.
    pub fn load(path: Option<&Path>) -> Result<Config, Error> {
        let (path, required) = match path {
            Some(path) => (path.to_path_buf(), true),
            None => match Config::default_path() {
                Some(path) => (path, false),
                None => return Ok(Config::default()),
            },
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
                return Ok(Config::default());
            }
            Err(err) => {
                return Err(Error::Config(format!(
                    "Could not read config file {}: {err}",
                    path.display()
                )));
            }
        };

        Config::parse(&text)
            .map_err(|err| Error::Config(format!("Invalid config file {}: {err}", path.display())))
    }

    /// Parse config text and validate it.
    pub fn parse(text: &str) -> Result<Config, String> {
        let config: Config = toml::from_str(text).map_err(|err| err.to_string())?;
        if !CODEX_EFFORTS.contains(&config.codex.effort.as_str()) {
            return Err(format!(
                "codex.effort must be one of {}, got \"{}\"",
                CODEX_EFFORTS.join(", "),
                config.codex.effort
            ));
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_all_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let config = Config::parse(
            r#"
            providers = ["claude"]
            poll_interval_seconds = 120

            [codex]
            model = "some-model"
            "#,
        )
        .unwrap();
        assert_eq!(config.providers, vec![Provider::Claude]);
        assert_eq!(config.poll_interval_seconds, 120);
        assert_eq!(config.codex.model, "some-model");
        assert_eq!(config.codex.effort, DEFAULT_CODEX_EFFORT);
        assert_eq!(config.thresholds, Thresholds::default());
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert!(Config::parse(r#"providers = ["nope"]"#).is_err());
    }

    #[test]
    fn invalid_effort_is_rejected() {
        assert!(Config::parse("[codex]\neffort = \"extreme\"").is_err());
    }
}
