use std::io::{self, BufRead, Write};
fn main() {
    assert_eq!(std::env::args().nth(1).as_deref(), Some("app-server"));
    let scenario = std::env::var("CODEX_HOME").unwrap();
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let init = lines.next().unwrap().unwrap();
    assert!(init.contains("initialize"));
    if scenario == "timeout" { std::thread::sleep(std::time::Duration::from_secs(60)); return; }
    if scenario == "eof" { return; }
    if scenario == "malformed" { println!("not json"); return; }
    if scenario == "oversized" { println!("{}", "x".repeat(1_048_577)); return; }
    if scenario == "rpc-error" { println!(r#"{{"id":1,"error":{{"message":"Login required"}}}}"#); return; }
    // A full pipe must not block the query.
    if scenario == "stderr" { eprint!("{}", "x".repeat(100_000)); }
    println!(r#"{{"method":"notification","params":{{}}}}"#);
    println!(r#"{{"id":99,"result":{{}}}}"#);
    println!(r#"{{"id":1,"result":{{}}}}"#);
    io::stdout().flush().unwrap();
    assert!(lines.next().unwrap().unwrap().contains("initialized"));
    let request = lines.next().unwrap().unwrap();
    assert!(request.contains("account/rateLimits/read"));
    assert!(!request.contains("thread/") && !request.contains("turn/"));
    if scenario == "client-request" {
        println!(r#"{{"id":7,"method":"unsupported/request","params":{{}}}}"#);
        io::stdout().flush().unwrap();
        let response=lines.next().unwrap().unwrap();
        assert!(response.contains("-32601"));
    }
    println!(r#"{{"id":2,"result":{{"rateLimitsByLimitId":{{"codex":{{"limitId":"codex","primary":{{"usedPercent":42,"windowDurationMins":300}}}}}}}}}}"#);
    io::stdout().flush().unwrap();
    // EOF is the requested graceful shutdown. No further RPCs are expected.
    assert!(lines.next().is_none());
}
