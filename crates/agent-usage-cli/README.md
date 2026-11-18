# agent-usage-cli

Show how much of your coding-agent subscription limits you have used, from
the command line. Supports Codex (ChatGPT subscription) and Claude Code
(Claude subscription).

**Disclosure:** This project was generated using Fable 5.2.

```
$ agent-usage-cli all
Codex (plus)
  Five-hour  5% used      resets in 3h 28m (3:20 AM)
  Weekly     1% used      resets in 6d 19h 26m (Mon Nov 23, 7:18 PM)

Claude Code (max)
  Five-hour  42% used     resets in 52m (11:49 PM)
  Weekly     31% used     resets in 3d 0h 2m (Thu Nov 19, 10:59 PM)
```

## Read this first

- **Inference and Claude queries use undocumented HTTP endpoints.** Codex's
  App Server strategy uses the documented `account/rateLimits/read` method
  through an installed official Codex CLI.
- **This is outside what the providers document.** Anthropic's Claude Code
  legal page says OAuth authentication is "designed to support ordinary use
  of Claude Code and other native Anthropic applications" and that Anthropic
  may enforce this without notice. Use this tool at your own risk.
- **Codex inference refreshes consume quota.** App Server refreshes read account
  limits without starting an inference turn. The sources can measure different
  quota groups; neither is assumed to belong exclusively to a particular model.
- **Authentication depends on strategy.** Inference and Claude read their configured
  credential files. App Server uses the official Codex login and its own credential
  refresh behavior. This monitor does not copy tokens between the two strategies.

## Install

You need the [Rust toolchain](https://rustup.rs).

```
cargo install --path crates/agent-usage-cli --locked
```

## Usage

### Codex strategies

```text
agent-usage-cli codex --source account
agent-usage-cli codex --source inference
agent-usage-cli codex --source both --json
agent-usage-cli codex --source account --codex-bin C:\path\codex.exe --codex-home C:\path\codex-home
agent-usage-cli all --codex-source both
```

Inference remains the default. Account mode requires an installed, logged-in
official Codex CLI supporting `account/rateLimits/read`. It inherits `CODEX_HOME`
unless `--codex-home` overrides it. It does not use `--auth-file` or the inference
token environment overrides. `--codex-bin` and `--codex-home` also work on `all`.
No failed account request silently falls back to inference.

Model, effort, and auth-file flags are rejected in account-only mode. Account
process flags are rejected in inference-only mode. In `both`, each applies to its
own source; accounts may differ if their authentication configurations differ.

Account/both JSON uses a `schema_version: 2` envelope with `sources` and `providers`
arrays. Each source retains its quota groups, identifiers, windows, enforcement
state, timestamp, and errors. Existing inference-only JSON is unchanged. A partial
failure retains successful readings and exits with code 1; invalid options exit 2.

`--source account --raw` returns the RPC result payload. `--source both --raw`
returns a versioned array of source results/errors. Default inference `--raw`
retains the existing `{status, headers}` shape. Raw output takes precedence over
`--json`. An account request has a 30-second deadline plus a short shutdown grace.

### Existing commands

```
agent-usage-cli codex  [--model M] [--effort E] [--auth-file PATH] [--raw]
agent-usage-cli claude [--credentials-file PATH] [--raw]
agent-usage-cli all    [--only codex,claude] [--codex-model M] [--codex-effort E]
                   [--codex-auth-file PATH] [--claude-credentials-file PATH]
```

Add `--json` to any command for machine-readable output. The exit code is
non-zero if any provider could not be queried.

- `all` queries every provider at once. `--only` picks providers and sets
  their order.
- `--model` and `--effort` choose what the Codex probe request uses
  (defaults: `gpt-6-luna`, `low`). Change the model if your account does not
  have the default.
- `claude --raw` prints the usage endpoint's unparsed response, which helps
  when its format changes.
- `codex --raw` prints JSON containing the HTTP `status` and unparsed
  `x-codex-*` response `headers`. Codex reports usage in these headers; the
  generated inference text is discarded. HTTP 429 quota responses are
  included. Other HTTP failures print an error and return a non-zero exit
  code. Like Claude's raw mode, this takes precedence over `--json`.

There is no config file and no built-in polling. To watch usage, wrap the
command in a poller, for example `watch -n 300 agent-usage-cli all`.

### Where credentials are found

| Provider | Order of lookup |
| --- | --- |
| Codex | `--auth-file`, then `CODEX_ACCESS_TOKEN` (with optional `CODEX_ACCOUNT_ID`), then `$CODEX_HOME/auth.json`, then `~/.codex/auth.json` |
| Claude Code | `--credentials-file`, then `$CLAUDE_CONFIG_DIR/.credentials.json`, then `~/.claude/.credentials.json` |

For inference and Claude, credentials kept in an OS keychain instead of a file are not supported,
apart from passing a Codex token through `CODEX_ACCESS_TOKEN`.

## JSON output

With the default inference strategy, one provider prints an object; `all` prints
an array of them. Account/both mode uses the versioned envelope described above.

```json
{
  "provider": "claude",
  "plan": "max",
  "metrics": [
    {
      "kind": "window",
      "label": "Five-hour",
      "duration_minutes": 300,
      "used_percent": 42.0,
      "remaining_percent": 58.0,
      "resets_at": 1794891000
    }
  ],
  "limit_reached": false,
  "details": {},
  "fetched_at": 1794887830,
  "error": null
}
```

- `metrics` holds entries of two kinds. `window` is a rate-limit window with
  a percent used and a reset time. `balance` is a credit balance with
  `used`, `limit`, `remaining`, and `currency`.
- Times are Unix epoch seconds.
- On failure, `error` holds a message and `metrics` is empty.

## As a library

The crate also builds a library, `agent_usage_cli`, with the same queries:
`agent_usage_cli::fetch(Provider, &Options)` and `agent_usage_cli::fetch_all`.

## Development and releases

See the [workspace README](../../README.md) for shared versioning and releases.

## License

MIT
