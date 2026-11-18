use std::{path::PathBuf, process::Command};
#[test]
fn account_and_partial_success_outputs() {
    let dir = std::env::temp_dir().join(format!("agent-usage-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let server = dir.join(if cfg!(windows) { "fake.exe" } else { "fake" });
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agent-usage-core/tests/fixtures/fake_app_server.rs");
    assert!(Command::new("rustc")
        .arg(fixture)
        .arg("-o")
        .arg(&server)
        .status()
        .unwrap()
        .success());
    let output = Command::new(env!("CARGO_BIN_EXE_agent-usage-cli"))
        .args(["codex", "--source", "both", "--json", "--codex-bin"])
        .arg(&server)
        .args(["--codex-home", "success", "--auth-file"])
        .arg(dir.join("missing.json"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(
        json["sources"][0]["groups"][0]["metrics"][0]["used_percent"],
        42.0
    );
    assert!(json["sources"][1]["error"].is_string());
    let raw = Command::new(env!("CARGO_BIN_EXE_agent-usage-cli"))
        .args(["codex", "--source", "account", "--raw", "--codex-bin"])
        .arg(&server)
        .args(["--codex-home", "success"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&raw.stdout).unwrap()["rateLimitsByLimitId"]
            .is_object()
    );
    for file in std::fs::read_dir(&dir).unwrap() {
        std::fs::remove_file(file.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn incompatible_options_are_rejected_before_querying() {
    for args in [
        vec!["codex", "--source", "account", "--model", "example"],
        vec!["codex", "--codex-home", "example"],
        vec!["all", "--codex-source", "account", "--codex-effort", "low"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-usage-cli"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}
