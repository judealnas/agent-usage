use agent_usage_core::codex_account::{fetch_raw, AccountOptions};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn process_protocol_and_cleanup() {
    let dir = std::env::temp_dir().join(format!("agent-usage-server-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let executable = dir.join(if cfg!(windows) { "fake.exe" } else { "fake" });
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_app_server.rs");
    assert!(Command::new("rustc")
        .arg(fixture)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    for scenario in [
        "success",
        "stderr",
        "client-request",
        "rpc-error",
        "eof",
        "malformed",
        "oversized",
        "timeout",
    ] {
        let options = AccountOptions {
            executable: executable.clone(),
            codex_home: Some(scenario.into()),
            timeout_seconds: 1,
        };
        let start = Instant::now();
        let result = fetch_raw(&options);
        if scenario == "success" || scenario == "stderr" || scenario == "client-request" {
            assert_eq!(
                result.unwrap()["rateLimitsByLimitId"]["codex"]["primary"]["usedPercent"],
                42
            );
        } else {
            let error = result.unwrap_err();
            if scenario == "timeout" {
                assert!(error.contains("timed out"));
            }
            if scenario == "rpc-error" {
                assert!(error.contains("Login required"));
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{scenario} exceeded its deadline"
        );
    }
    // On Windows an executable cannot be deleted while its process is running.
    std::fs::remove_file(&executable).unwrap();
    for file in std::fs::read_dir(&dir).unwrap() {
        std::fs::remove_file(file.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn missing_executable_is_actionable() {
    let error = fetch_raw(&AccountOptions {
        executable: "nonexistent-agent-usage-test-executable".into(),
        ..Default::default()
    })
    .unwrap_err();
    assert!(error.contains("executable path"));
}
