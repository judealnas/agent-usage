//! Per-provider options. Callers build these directly (the CLI from its
//! flags, a GUI from its own settings); this crate reads no config file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-luna";
pub const DEFAULT_CODEX_EFFORT: &str = "low";
pub const CODEX_EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub codex: CodexOptions,
    pub claude: ClaudeOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexOptions {
    /// Model used for the probe request.
    pub model: String,
    /// Reasoning effort used for the probe request.
    pub effort: String,
    /// Path to a Codex `auth.json`. Overrides the environment lookups.
    pub auth_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeOptions {
    /// Path to Claude Code's `.credentials.json`.
    pub credentials_file: Option<PathBuf>,
}

impl Default for CodexOptions {
    fn default() -> Self {
        CodexOptions {
            model: DEFAULT_CODEX_MODEL.to_string(),
            effort: DEFAULT_CODEX_EFFORT.to_string(),
            auth_file: None,
        }
    }
}
