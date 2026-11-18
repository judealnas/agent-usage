//! Source-aware queries without changing the original inference-only API.
use crate::{codex, codex_account, CodexOptions, Metric, UsageReport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSource {
    Account,
    Inference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaGroup {
    pub key: String,
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub plan: Option<String>,
    pub metrics: Vec<Metric>,
    pub ordinary_usage_allowed: Option<bool>,
    pub spend_control_reached: Option<bool>,
    pub rate_limit_reached_type: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReport {
    pub source: CodexSource,
    pub fetched_at: i64,
    pub groups: Vec<QuotaGroup>,
    pub probe_model: Option<String>,
    pub error: Option<String>,
}
impl SourceReport {
    pub fn failed(source: CodexSource, error: impl ToString) -> Self {
        Self {
            source,
            fetched_at: chrono::Utc::now().timestamp(),
            groups: vec![],
            probe_model: None,
            error: Some(error.to_string()),
        }
    }
    pub fn is_ok(&self) -> bool {
        self.error.is_none() && self.groups.iter().all(|g| g.error.is_none())
    }
    pub fn inference(report: UsageReport, model: &str) -> Self {
        let id = report.details.get("active_limit").cloned();
        Self {
            source: CodexSource::Inference,
            fetched_at: report.fetched_at,
            probe_model: Some(model.into()),
            error: report.error,
            groups: vec![QuotaGroup {
                key: id.clone().unwrap_or_else(|| "inference".into()),
                limit_id: id,
                limit_name: None,
                plan: report.plan,
                metrics: report.metrics,
                ordinary_usage_allowed: None,
                spend_control_reached: None,
                rate_limit_reached_type: report
                    .details
                    .get("rate_limit_reached_type")
                    .cloned()
                    .or_else(|| report.limit_reached.then(|| "limit_reached".into())),
                error: None,
            }],
        }
    }
}

pub fn fetch(
    source: CodexSource,
    inference: &CodexOptions,
    account: &codex_account::AccountOptions,
) -> SourceReport {
    match source {
        CodexSource::Inference => match codex::fetch(inference) {
            Ok(report) => SourceReport::inference(report, &inference.model),
            Err(err) => SourceReport::failed(source, err),
        },
        CodexSource::Account => match codex_account::fetch_raw(account) {
            Ok(value) => codex_account::parse(&value),
            Err(err) => SourceReport::failed(source, err),
        },
    }
}

pub fn raw(
    source: CodexSource,
    inference: &CodexOptions,
    account: &codex_account::AccountOptions,
) -> Result<Value, String> {
    match source {
        CodexSource::Account => codex_account::fetch_raw(account),
        CodexSource::Inference => codex::fetch_raw(inference)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())),
    }
}
