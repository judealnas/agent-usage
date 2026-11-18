//! The app's settings file. It is owned by the app and rewritten on every
//! change; it is not meant for hand editing.

use std::path::{Path, PathBuf};

use agent_usage_core::options::CODEX_EFFORTS;
use agent_usage_core::{ClaudeOptions, CodexOptions, Options, Provider};
use serde::{Deserialize, Serialize};

pub const DEFAULT_INTERVAL_SECONDS: u64 = 300;
pub const MIN_INTERVAL_SECONDS: u64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Every known provider, in display order.
    pub providers: Vec<ProviderSettings>,
    pub codex: CodexOptions,
    pub claude: ClaudeOptions,
    pub thresholds: Thresholds,
    /// Warn when a provider is set to poll faster than once a minute.
    #[serde(alias = "warn_fast_codex_polling")]
    pub warn_fast_polling: bool,
    /// Keep the window above other windows.
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub id: Provider,
    #[serde(default = "yes")]
    pub shown: bool,
    #[serde(default = "yes")]
    pub auto_refresh: bool,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
}

/// Percent used at which the green-to-red bar gradient reaches red.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub critical: f64,
}

/// The part of the settings edited in the settings view.
#[derive(Debug, Clone, Deserialize)]
pub struct Preferences {
    pub codex: CodexOptions,
    pub claude: ClaudeOptions,
    pub thresholds: Thresholds,
    pub warn_fast_polling: bool,
}

fn yes() -> bool {
    true
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECONDS
}

impl ProviderSettings {
    fn new(id: Provider) -> ProviderSettings {
        ProviderSettings {
            id,
            shown: true,
            auto_refresh: true,
            interval_seconds: DEFAULT_INTERVAL_SECONDS,
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { critical: 90.0 }
    }
}

impl Thresholds {
    fn validate(&self) -> Result<(), String> {
        if !self.critical.is_finite() || !(1.0..=100.0).contains(&self.critical) {
            return Err("The red threshold must be between 1 and 100 percent.".to_string());
        }
        Ok(())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            providers: Provider::ALL
                .into_iter()
                .map(ProviderSettings::new)
                .collect(),
            codex: CodexOptions::default(),
            claude: ClaudeOptions::default(),
            thresholds: Thresholds::default(),
            warn_fast_polling: true,
            pinned: true,
        }
    }
}

fn blank_to_none(path: &mut Option<PathBuf>) {
    if path
        .as_ref()
        .is_some_and(|p| p.as_os_str().to_string_lossy().trim().is_empty())
    {
        *path = None;
    }
}

impl Settings {
    /// Where the settings file lives: `%APPDATA%\agent-usage-gui\settings.json`
    /// on Windows, and the platform's config directory elsewhere.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-usage-gui")
            .join("settings.json")
    }

    /// Load settings. A missing or unreadable file means defaults.
    pub fn load(path: &Path) -> Settings {
        let mut settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();
        settings.normalize();
        settings
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let failed = |err: std::io::Error| format!("Could not save settings: {err}");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        std::fs::write(path, text).map_err(failed)
    }

    /// Repair anything a stale or damaged file could contain: duplicate
    /// providers, providers added since the file was written, intervals
    /// below the minimum, and invalid option values.
    pub fn normalize(&mut self) {
        let mut seen = Vec::new();
        self.providers.retain(|p| {
            let first = !seen.contains(&p.id);
            seen.push(p.id);
            first
        });
        for provider in Provider::ALL {
            if !seen.contains(&provider) {
                self.providers.push(ProviderSettings::new(provider));
            }
        }
        for provider in &mut self.providers {
            provider.interval_seconds = provider.interval_seconds.max(MIN_INTERVAL_SECONDS);
        }

        if self.thresholds.validate().is_err() {
            self.thresholds = Thresholds::default();
        }
        if self.codex.model.trim().is_empty() {
            self.codex.model = CodexOptions::default().model;
        }
        if !CODEX_EFFORTS.contains(&self.codex.effort.as_str()) {
            self.codex.effort = CodexOptions::default().effort;
        }
        blank_to_none(&mut self.codex.auth_file);
        blank_to_none(&mut self.claude.credentials_file);
    }

    pub fn options(&self) -> Options {
        Options {
            codex: self.codex.clone(),
            claude: self.claude.clone(),
        }
    }

    pub fn provider(&self, id: Provider) -> Option<&ProviderSettings> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn shown(&self) -> Vec<Provider> {
        self.providers
            .iter()
            .filter(|p| p.shown)
            .map(|p| p.id)
            .collect()
    }

    pub fn set_auto_refresh(
        &mut self,
        id: Provider,
        auto_refresh: bool,
        interval_seconds: u64,
    ) -> Result<(), String> {
        if interval_seconds < MIN_INTERVAL_SECONDS {
            return Err("The refresh interval must be at least 1 second.".to_string());
        }
        let provider = self
            .providers
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Unknown provider.")?;
        provider.auto_refresh = auto_refresh;
        provider.interval_seconds = interval_seconds;
        Ok(())
    }

    /// Show or hide a provider. A newly shown provider moves to the bottom.
    pub fn set_shown(&mut self, id: Provider, shown: bool) -> Result<(), String> {
        let index = self
            .providers
            .iter()
            .position(|p| p.id == id)
            .ok_or("Unknown provider.")?;
        let mut provider = self.providers.remove(index);
        let was_shown = provider.shown;
        provider.shown = shown;
        if shown && !was_shown {
            self.providers.push(provider);
        } else {
            self.providers.insert(index, provider);
        }
        Ok(())
    }

    /// Reorder providers. Any provider missing from `order` keeps its
    /// relative position after the listed ones.
    pub fn set_order(&mut self, order: &[Provider]) {
        self.providers.sort_by_key(|p| {
            order
                .iter()
                .position(|&id| id == p.id)
                .unwrap_or(usize::MAX)
        });
    }

    pub fn apply_preferences(&mut self, preferences: Preferences) -> Result<(), String> {
        if preferences.codex.model.trim().is_empty() {
            return Err("The Codex model cannot be empty.".to_string());
        }
        if !CODEX_EFFORTS.contains(&preferences.codex.effort.as_str()) {
            return Err(format!(
                "The Codex effort must be one of: {}.",
                CODEX_EFFORTS.join(", ")
            ));
        }
        preferences.thresholds.validate()?;

        self.codex = preferences.codex;
        self.codex.model = self.codex.model.trim().to_string();
        self.claude = preferences.claude;
        self.thresholds = preferences.thresholds;
        self.warn_fast_polling = preferences.warn_fast_polling;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(settings: &Settings) -> Vec<Provider> {
        settings.providers.iter().map(|p| p.id).collect()
    }

    #[test]
    fn defaults_show_every_provider_at_five_minutes() {
        let settings = Settings::default();
        assert_eq!(ids(&settings), Provider::ALL.to_vec());
        assert!(settings
            .providers
            .iter()
            .all(|p| p.shown && p.auto_refresh && p.interval_seconds == 300));
        assert!(settings.pinned);
        assert!(settings.warn_fast_polling);
    }

    #[test]
    fn a_newly_known_provider_is_appended_as_shown() {
        let mut settings: Settings = serde_json::from_str(
            r#"{"providers":[{"id":"claude","shown":false,"auto_refresh":false,"interval_seconds":30}]}"#,
        )
        .unwrap();
        settings.normalize();
        assert_eq!(ids(&settings), vec![Provider::Claude, Provider::Codex]);
        assert!(!settings.providers[0].shown);
        assert!(settings.providers[1].shown);
    }

    #[test]
    fn normalize_repairs_bad_values() {
        let mut settings: Settings = serde_json::from_str(
            r#"{
                "providers":[{"id":"codex","interval_seconds":0},{"id":"codex"}],
                "codex":{"model":" ","effort":"extreme","auth_file":""},
                "thresholds":{"warn":95,"critical":120}
            }"#,
        )
        .unwrap();
        settings.normalize();
        assert_eq!(ids(&settings), vec![Provider::Codex, Provider::Claude]);
        assert_eq!(settings.providers[0].interval_seconds, 1);
        assert_eq!(settings.codex, CodexOptions::default());
        assert_eq!(settings.thresholds, Thresholds::default());
    }

    #[test]
    fn a_missing_or_corrupt_file_means_defaults() {
        assert_eq!(
            Settings::load(Path::new("definitely/not/here.json")),
            Settings::default()
        );
    }

    #[test]
    fn intervals_below_one_second_are_rejected() {
        let mut settings = Settings::default();
        assert!(settings.set_auto_refresh(Provider::Codex, true, 0).is_err());
        assert!(settings.set_auto_refresh(Provider::Codex, false, 1).is_ok());
        let codex = settings.provider(Provider::Codex).unwrap();
        assert!(!codex.auto_refresh);
        assert_eq!(codex.interval_seconds, 1);
    }

    #[test]
    fn showing_a_hidden_provider_moves_it_to_the_bottom() {
        let mut settings = Settings::default();
        settings.set_shown(Provider::Codex, false).unwrap();
        assert_eq!(ids(&settings), vec![Provider::Codex, Provider::Claude]);
        assert_eq!(settings.shown(), vec![Provider::Claude]);
        settings.set_shown(Provider::Codex, true).unwrap();
        assert_eq!(ids(&settings), vec![Provider::Claude, Provider::Codex]);
    }

    #[test]
    fn set_order_reorders_and_keeps_unlisted_last() {
        let mut settings = Settings::default();
        settings.set_order(&[Provider::Claude, Provider::Codex]);
        assert_eq!(ids(&settings), vec![Provider::Claude, Provider::Codex]);
        settings.set_order(&[Provider::Codex]);
        assert_eq!(ids(&settings), vec![Provider::Codex, Provider::Claude]);
    }

    #[test]
    fn legacy_red_threshold_is_preserved() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"thresholds":{"warn":70,"critical":85}}"#).unwrap();
        settings.normalize();
        assert_eq!(settings.thresholds.critical, 85.0);
        let json = serde_json::to_value(&settings).unwrap();
        assert!(json["thresholds"].get("warn").is_none());
    }

    #[test]
    fn red_threshold_requires_a_finite_percentage() {
        for critical in [0.0, -1.0, 101.0, f64::NAN, f64::INFINITY] {
            assert!(Thresholds { critical }.validate().is_err());
        }
        for critical in [1.0, 50.5, 100.0] {
            assert!(Thresholds { critical }.validate().is_ok());
        }
    }

    #[test]
    fn preferences_are_validated() {
        let mut settings = Settings::default();
        let preferences = |critical: f64, effort: &str| Preferences {
            codex: CodexOptions {
                effort: effort.to_string(),
                ..CodexOptions::default()
            },
            claude: ClaudeOptions::default(),
            thresholds: Thresholds { critical },
            warn_fast_polling: false,
        };
        assert!(settings.apply_preferences(preferences(0.0, "low")).is_err());
        assert!(settings
            .apply_preferences(preferences(120.0, "low"))
            .is_err());
        assert!(settings
            .apply_preferences(preferences(80.0, "extreme"))
            .is_err());
        assert_eq!(settings, Settings::default());

        settings
            .apply_preferences(preferences(80.0, "high"))
            .unwrap();
        assert_eq!(settings.thresholds.critical, 80.0);
        assert_eq!(settings.codex.effort, "high");
        assert!(!settings.warn_fast_polling);
    }
}
