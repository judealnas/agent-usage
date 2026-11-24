//! Provider instances and migration from the original per-provider settings.
use super::settings::{Settings as LegacySettings, Thresholds};
use agent_usage_core::{
    codex_account::AccountOptions, options::CODEX_EFFORTS, ClaudeOptions, CodexOptions, Provider,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Configuration {
    CodexInference { options: CodexOptions },
    CodexAccount { options: AccountOptions },
    Claude { options: ClaudeOptions },
}
impl Configuration {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CodexInference { .. } => "Codex — Inference",
            Self::CodexAccount { .. } => "Codex — App Server",
            Self::Claude { .. } => "Claude",
        }
    }
    fn normalize(&mut self) {
        fn path(p: &mut Option<PathBuf>) {
            if p.as_ref().is_some_and(|v| v.as_os_str().is_empty()) {
                *p = None;
            }
            if let Some(value) = p {
                if let Ok(rest) = value.strip_prefix("~") {
                    *value = dirs::home_dir().unwrap_or_default().join(rest);
                }
            }
        }
        match self {
            Self::CodexInference { options } => {
                options.model = options.model.trim().into();
                path(&mut options.auth_file);
            }
            Self::CodexAccount { options } => path(&mut options.codex_home),
            Self::Claude { options } => path(&mut options.credentials_file),
        }
    }
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::CodexInference { options }
                if options.model.trim().is_empty()
                    || !CODEX_EFFORTS.contains(&options.effort.as_str()) =>
            {
                Err("Choose a model and a valid reasoning effort.".into())
            }
            Self::CodexAccount { options }
                if options.executable.as_os_str().is_empty()
                    || !(1..=120).contains(&options.timeout_seconds) =>
            {
                Err("Set a Codex executable and a timeout between 1 and 120 seconds.".into())
            }
            _ => Ok(()),
        }
    }
    fn identity(&self) -> String {
        let home = dirs::home_dir().unwrap_or_default();
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        match self {
            Self::CodexInference { options } => {
                if options.auth_file.is_none()
                    && std::env::var("CODEX_ACCESS_TOKEN").is_ok_and(|s| !s.is_empty())
                {
                    return "inference:environment".into();
                }
                format!(
                    "inference:{}",
                    path_key(
                        &options
                            .auth_file
                            .clone()
                            .unwrap_or_else(|| codex_home.join("auth.json"))
                    )
                )
            }
            Self::CodexAccount { options } => format!(
                "account:{}",
                path_key(&options.codex_home.clone().unwrap_or(codex_home))
            ),
            Self::Claude { options } => format!(
                "claude:{}",
                path_key(&options.credentials_file.clone().unwrap_or_else(|| {
                    std::env::var_os("CLAUDE_CONFIG_DIR")
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| home.join(".claude"))
                        .join(".credentials.json")
                }))
            ),
        }
    }
}
fn path_key(path: &Path) -> String {
    let expanded = if let Ok(rest) = path.strip_prefix("~") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        path.to_path_buf()
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().unwrap_or_default().join(expanded)
    };
    let canonical = std::fs::canonicalize(&absolute).unwrap_or_else(|_| {
        let mut result = PathBuf::new();
        for part in absolute.components() {
            match part {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    result.pop();
                }
                _ => result.push(part),
            }
        }
        result
    });
    let text = canonical.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        text.trim_start_matches(r"\\?\")
            .replace('/', r"\")
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        text
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: Option<String>,
    pub config: Configuration,
    pub generation: u64,
    pub shown: bool,
    pub auto_refresh: bool,
    pub interval_seconds: u64,
    #[serde(default)]
    pub tray_enabled: bool,
    #[serde(default)]
    pub tray_group: Option<String>,
}
impl Entry {
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or_else(|| self.config.label())
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct EntryDraft {
    pub id: Option<String>,
    pub name: Option<String>,
    pub config: Configuration,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u32,
    pub next_id: u64,
    pub providers: Vec<Entry>,
    pub thresholds: Thresholds,
    pub warn_fast_polling: bool,
    pub pinned: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}
fn default_true() -> bool {
    true
}
#[derive(Deserialize)]
pub struct Preferences {
    pub thresholds: Thresholds,
    pub warn_fast_polling: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 2,
            next_id: 1,
            providers: Vec::new(),
            thresholds: Thresholds::default(),
            warn_fast_polling: true,
            pinned: true,
            close_to_tray: true,
        }
    }
}
impl Settings {
    fn migrate(mut old: LegacySettings) -> Self {
        old.normalize();
        let providers = old
            .providers
            .iter()
            .enumerate()
            .map(|(index, p)| Entry {
                id: format!("entry-{}", index + 1),
                name: None,
                generation: 0,
                shown: p.shown,
                auto_refresh: p.auto_refresh,
                interval_seconds: p.interval_seconds,
                tray_enabled: false,
                tray_group: None,
                config: match p.id {
                    Provider::Codex => Configuration::CodexInference {
                        options: old.codex.clone(),
                    },
                    Provider::Claude => Configuration::Claude {
                        options: old.claude.clone(),
                    },
                },
            })
            .collect();
        Self {
            schema_version: 2,
            next_id: 3,
            providers,
            thresholds: old.thresholds,
            warn_fast_polling: old.warn_fast_polling,
            pinned: old.pinned,
            close_to_tray: true,
        }
    }
    pub fn default_path() -> PathBuf {
        LegacySettings::default_path()
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if value.get("schema_version").is_none() {
            return serde_json::from_value(value)
                .map(Self::migrate)
                .map_err(|e| e.to_string());
        }
        let result: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if result.schema_version != 2 {
            return Err("Unsupported settings version.".into());
        }
        Ok(result)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_json(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            path,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn provider(&self, id: &str) -> Option<&Entry> {
        self.providers.iter().find(|p| p.id == id)
    }
    pub fn monitored(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|p| p.shown || p.tray_enabled)
            .map(|p| p.id.clone())
            .collect()
    }
    pub fn upsert(&mut self, mut draft: EntryDraft) -> Result<String, String> {
        draft.config.normalize();
        draft.config.validate()?;
        let key = draft.config.identity();
        if let Some(existing) = self
            .providers
            .iter()
            .find(|p| Some(&p.id) != draft.id.as_ref() && p.config.identity() == key)
        {
            return Err(format!(
                "This credential source and strategy are already used by {}.",
                existing.label()
            ));
        }
        let name = draft
            .name
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if let Some(id) = draft.id {
            let entry = self
                .providers
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("Unknown entry.")?;
            if entry.config != draft.config {
                entry.generation += 1;
            }
            entry.name = name;
            entry.config = draft.config;
            Ok(id)
        } else {
            while self.provider(&format!("entry-{}", self.next_id)).is_some() {
                self.next_id += 1;
            }
            let id = format!("entry-{}", self.next_id);
            self.next_id += 1;
            self.providers.push(Entry {
                id: id.clone(),
                name,
                config: draft.config,
                generation: 0,
                shown: true,
                auto_refresh: true,
                interval_seconds: 300,
                tray_enabled: false,
                tray_group: None,
            });
            Ok(id)
        }
    }
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let index = self
            .providers
            .iter()
            .position(|p| p.id == id)
            .ok_or("Unknown entry.")?;
        self.providers.remove(index);
        Ok(())
    }
    pub fn set_auto_refresh(
        &mut self,
        id: &str,
        auto_refresh: bool,
        interval_seconds: u64,
    ) -> Result<(), String> {
        if interval_seconds == 0 {
            return Err("The refresh interval must be at least 1 second.".into());
        }
        let p = self
            .providers
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Unknown entry.")?;
        p.auto_refresh = auto_refresh;
        p.interval_seconds = interval_seconds;
        Ok(())
    }
    pub fn set_shown(&mut self, id: &str, shown: bool) -> Result<(), String> {
        let index = self
            .providers
            .iter()
            .position(|p| p.id == id)
            .ok_or("Unknown entry.")?;
        let mut entry = self.providers.remove(index);
        let changed = entry.shown != shown;
        entry.shown = shown;
        if changed && shown {
            self.providers.push(entry);
        } else {
            self.providers.insert(index, entry);
        }
        Ok(())
    }
    pub fn set_order(&mut self, order: &[String]) {
        self.providers.sort_by_key(|p| {
            order
                .iter()
                .position(|id| id == &p.id)
                .unwrap_or(usize::MAX)
        });
    }
    pub fn apply_preferences(&mut self, p: Preferences) -> Result<(), String> {
        if !p.thresholds.critical.is_finite() || !(1.0..=100.0).contains(&p.thresholds.critical) {
            return Err("Red threshold must be between 1 and 100 percent.".into());
        }
        self.thresholds = p.thresholds;
        self.warn_fast_polling = p.warn_fast_polling;
        self.close_to_tray = p.close_to_tray;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn configured_settings() -> Settings {
        Settings::migrate(LegacySettings::default())
    }

    #[test]
    fn missing_settings_starts_empty_and_can_save_the_first_provider() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let mut settings = Settings::load(&path).unwrap();
        assert!(settings.providers.is_empty());
        assert!(settings.monitored().is_empty());

        let id = settings
            .upsert(EntryDraft {
                id: None,
                name: None,
                config: Configuration::CodexInference {
                    options: Default::default(),
                },
            })
            .unwrap();
        assert_eq!(id, "entry-1");
        settings.save(&path).unwrap();
        let reloaded = Settings::load(&path).unwrap();
        assert_eq!(reloaded.providers, settings.providers);
        assert_eq!(reloaded.monitored(), [id]);
    }

    #[test]
    fn tray_defaults_migrate_and_hidden_entries_keep_monitoring() {
        let mut value = serde_json::to_value(configured_settings()).unwrap();
        value.as_object_mut().unwrap().remove("close_to_tray");
        for p in value["providers"].as_array_mut().unwrap() {
            p.as_object_mut().unwrap().remove("tray_enabled");
            p.as_object_mut().unwrap().remove("tray_group");
        }
        let mut settings = Settings::from_json(&value.to_string()).unwrap();
        assert!(settings.close_to_tray);
        assert!(settings.providers.iter().all(|p| !p.tray_enabled));
        settings.providers[0].shown = false;
        let id = settings.providers[0].id.clone();
        assert!(!settings.monitored().contains(&id));
        settings.providers[0].tray_enabled = true;
        settings.providers[0].tray_group = Some("work".into());
        settings.close_to_tray = false;
        assert!(settings.monitored().contains(&id));
        let loaded = Settings::from_json(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!loaded.close_to_tray);
        assert_eq!(loaded.providers, settings.providers);
    }
    #[test]
    fn migration_preserves_configuration() {
        let s=Settings::from_json(r#"{"codex":{"model":"custom","auth_file":"account.json"},"thresholds":{"critical":82}}"#).unwrap();
        assert_eq!(s.thresholds.critical, 82.0);
        assert!(
            matches!(&s.providers[0].config,Configuration::CodexInference{options} if options.model=="custom")
        );
        assert_eq!(
            Settings::from_json(&serde_json::to_string(&s).unwrap())
                .unwrap()
                .providers,
            s.providers
        );
    }
    #[test]
    fn duplicates_ignore_name_and_model_but_allow_other_strategy() {
        let mut s = configured_settings();
        let mut config = s.providers[0].config.clone();
        if let Configuration::CodexInference { options } = &mut config {
            options.model = "different".into();
        }
        assert!(s
            .upsert(EntryDraft {
                id: None,
                name: Some("Another".into()),
                config
            })
            .is_err());
        assert!(s
            .upsert(EntryDraft {
                id: None,
                name: None,
                config: Configuration::CodexAccount {
                    options: AccountOptions::default()
                }
            })
            .is_ok());
    }
    #[test]
    fn rename_keeps_generation_and_removal_survives_reload() {
        let mut s = configured_settings();
        let e = s.providers[0].clone();
        s.upsert(EntryDraft {
            id: Some(e.id.clone()),
            name: Some("Work".into()),
            config: e.config,
        })
        .unwrap();
        assert_eq!(s.providers[0].generation, 0);
        s.remove(&e.id).unwrap();
        assert_eq!(
            Settings::from_json(&serde_json::to_string(&s).unwrap())
                .unwrap()
                .providers
                .len(),
            1
        );
    }
    #[test]
    fn equivalent_paths_match() {
        assert_eq!(path_key(Path::new("a/../b")), path_key(Path::new("b")));
    }

    #[test]
    fn explicit_default_credentials_are_duplicates() {
        let mut settings = configured_settings();
        let home = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".claude"));
        let draft = EntryDraft {
            id: None,
            name: None,
            config: Configuration::Claude {
                options: ClaudeOptions {
                    credentials_file: Some(home.join(".credentials.json")),
                },
            },
        };
        assert!(settings.upsert(draft).unwrap_err().contains("already used"));
    }

    #[test]
    fn config_change_advances_generation_and_preserves_card_interval() {
        let mut settings = configured_settings();
        let entry = settings.providers[0].clone();
        settings.set_auto_refresh(&entry.id, false, 17).unwrap();
        let mut config = entry.config;
        if let Configuration::CodexInference { options } = &mut config {
            options.model = "different-model".into();
        }
        settings
            .upsert(EntryDraft {
                id: Some(entry.id.clone()),
                name: None,
                config,
            })
            .unwrap();
        let edited = settings.provider(&entry.id).unwrap();
        assert_eq!(edited.generation, 1);
        assert_eq!(edited.interval_seconds, 17);
        assert!(!edited.auto_refresh);
    }
}
