//! Read account quota through an installed Codex App Server, without inference.
use crate::{
    sources::{CodexSource, QuotaGroup, SourceReport},
    Metric,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountOptions {
    pub executable: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub timeout_seconds: u64,
}
impl Default for AccountOptions {
    fn default() -> Self {
        Self {
            executable: "codex".into(),
            codex_home: None,
            timeout_seconds: 30,
        }
    }
}

pub fn fetch_raw(options: &AccountOptions) -> Result<Value, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(query(options))
}

async fn response(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    input: &mut tokio::process::ChildStdin,
    id: u64,
) -> Result<Value, String> {
    loop {
        // take() bounds allocation even if the server never sends a newline.
        let mut line = Vec::new();
        let count = reader
            .take(1_048_577)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("Codex App Server exited before returning quota.".into());
        }
        if count > 1_048_576 {
            return Err("App Server message exceeds 1 MiB.".into());
        }
        let message: Value =
            serde_json::from_slice(&line).map_err(|e| format!("Invalid App Server JSON: {e}"))?;
        if message.get("method").is_some() {
            if message.get("id").is_some() {
                let reply = json!({"id":message["id"],"error":{"code":-32601,"message":"This usage monitor does not support client requests. Use the official Codex login."}});
                input
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            continue;
        }
        if message["id"] != id {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(format!(
                "Codex App Server: {error}. Check your Codex login and CLI version."
            ));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| "App Server response has no result.".into());
    }
}

async fn query(options: &AccountOptions) -> Result<Value, String> {
    let mut command = Command::new(resolve_executable(&options.executable));
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(home) = &options.codex_home {
        command.env("CODEX_HOME", home);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| {
        format!("Could not start Codex App Server: {e}. Install Codex or set its executable path.")
    })?;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut stderr = child.stderr.take().unwrap();
    // Drain, but do not accumulate or expose potentially sensitive diagnostics.
    let drain = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
    });
    let result = tokio::time::timeout(Duration::from_secs(options.timeout_seconds.clamp(1, 120)), async {
        let init = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"agent_usage","version":env!("CARGO_PKG_VERSION")}}});
        input.write_all(format!("{init}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        response(&mut output, &mut input, 1).await?;
        input.write_all(b"{\"method\":\"initialized\",\"params\":{}}\n{\"id\":2,\"method\":\"account/rateLimits/read\"}\n").await.map_err(|e| e.to_string())?;
        response(&mut output, &mut input, 2).await
    }).await.unwrap_or_else(|_| Err("Codex App Server timed out.".into()));
    drop(input);
    drop(output);
    if tokio::time::timeout(Duration::from_millis(500), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    drain.abort();
    let _ = drain.await;
    result
}

pub fn parse(value: &Value) -> SourceReport {
    let mut report = SourceReport::failed(CodexSource::Account, "No account quota returned.");
    if !value.is_object()
        || (value.get("rateLimitsByLimitId").is_none_or(Value::is_null)
            && !value.get("rateLimits").is_some_and(Value::is_object))
    {
        return report;
    }
    let entries: Vec<(String, &Value)> = match value.get("rateLimitsByLimitId") {
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        None | Some(Value::Null) => value
            .get("rateLimits")
            .filter(|v| v.is_object())
            .map(|v| vec![(v["limitId"].as_str().unwrap_or("legacy").into(), v)])
            .unwrap_or_default(),
        _ => {
            report.error = Some("Invalid rateLimitsByLimitId map.".into());
            return report;
        }
    };
    report.error = None;
    for (key, value) in entries {
        let mut group = QuotaGroup {
            key,
            limit_id: value["limitId"].as_str().map(str::to_owned),
            limit_name: value["limitName"].as_str().map(str::to_owned),
            plan: value["planType"].as_str().map(str::to_owned),
            metrics: vec![],
            ordinary_usage_allowed: value["ordinaryUsageAllowed"].as_bool(),
            spend_control_reached: value["spendControlReached"].as_bool(),
            rate_limit_reached_type: value["rateLimitReachedType"].as_str().map(str::to_owned),
            error: None,
        };
        if !value.is_object() {
            group.error = Some("Invalid quota group.".into());
        }
        for name in ["primary", "secondary"] {
            let window = &value[name];
            if window.is_null() {
                continue;
            }
            let Some(used) = window["usedPercent"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
            else {
                group.error = Some(format!("Invalid {name} usage window."));
                continue;
            };
            let duration = window["windowDurationMins"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok());
            let label = match duration {
                Some(300) => "Five-hour".into(),
                Some(10080) => "Weekly".into(),
                Some(n) => format!("{n}-minute"),
                None => name.into(),
            };
            group.metrics.push(Metric::window(
                label,
                duration,
                used,
                window["resetsAt"].as_i64(),
            ));
        }
        report.groups.push(group);
    }
    report
}

/// Resolve npm shims to the bundled native binary, avoiding a shell and orphaned
/// wrapper processes. Unknown layouts retain the supplied path and fail clearly.
fn resolve_executable(path: &std::path::Path) -> PathBuf {
    fn native_under(root: &std::path::Path, depth: usize) -> Option<PathBuf> {
        if depth == 0 {
            return None;
        }
        let entries = std::fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let candidate = entry.path();
            if entry.file_type().ok()?.is_dir() {
                if let Some(found) = native_under(&candidate, depth - 1) {
                    return Some(found);
                }
            } else if candidate
                .file_name()
                .is_some_and(|s| s == if cfg!(windows) { "codex.exe" } else { "codex" })
            {
                return Some(candidate);
            }
        }
        None
    }
    let candidates: Vec<PathBuf> = if path.components().count() == 1 && !path.is_absolute() {
        std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .flat_map(|dir| {
                        let base = dir.join(path);
                        if cfg!(windows) {
                            vec![base.with_extension("exe"), base.with_extension("cmd"), base]
                        } else {
                            vec![base]
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![path.to_path_buf()]
    };
    if let Some(candidate) = candidates.into_iter().find(|p| p.is_file()) {
        if candidate.extension().is_some_and(|s| s == "exe") {
            return candidate;
        }
        let parent = candidate
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let scope = parent.join("node_modules/@openai");
        // Match the platform package before the package's legacy vendor layout.
        let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => "codex-win32-x64",
            ("windows", "aarch64") => "codex-win32-arm64",
            ("linux", "x86_64") => "codex-linux-x64",
            ("linux", "aarch64") => "codex-linux-arm64",
            ("macos", "x86_64") => "codex-darwin-x64",
            ("macos", "aarch64") => "codex-darwin-arm64",
            _ => "codex",
        };
        for root in [scope.join(platform), scope.join("codex/vendor")] {
            if let Some(native) = native_under(&root, 6) {
                return native;
            }
        }
        return candidate;
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_groups_and_duration_labels() {
        let r = parse(
            &json!({"rateLimitsByLimitId":{"codex":{"secondary":{"usedPercent":42,"windowDurationMins":300}},"premium":{"ordinaryUsageAllowed":false}}}),
        );
        assert_eq!(r.groups.len(), 2);
        assert!(matches!(&r.groups[0].metrics[0],Metric::Window{label,..} if label=="Five-hour"));
        assert_eq!(r.groups[0].ordinary_usage_allowed, None);
        assert_eq!(r.groups[1].ordinary_usage_allowed, Some(false));
    }
    #[test]
    fn empty_map_does_not_revive_legacy() {
        assert!(
            parse(&json!({"rateLimitsByLimitId":{},"rateLimits":{"limitId":"old"}}))
                .groups
                .is_empty()
        );
        assert_eq!(
            parse(&json!({"rateLimits":{"limitId":"old"}})).groups[0].key,
            "old"
        );
    }
    #[test]
    fn bad_group_preserves_successes() {
        let r = parse(
            &json!({"rateLimitsByLimitId":{"bad":false,"good":{"primary":{"usedPercent":5}}}}),
        );
        assert!(!r.is_ok());
        assert_eq!(r.groups[1].metrics.len(), 1);
    }
}
