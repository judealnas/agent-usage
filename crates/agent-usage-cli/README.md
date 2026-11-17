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

- **Both providers are queried through undocumented endpoints**, using the
  login tokens the official tools already store on your machine (the Codex
  CLI's `auth.json` and Claude Code's `.credentials.json`). These endpoints
  can change or stop working without notice.
- **This is outside what the providers document.** Anthropic's Claude Code
  legal page says OAuth authentication is "designed to support ordinary use
  of Claude Code and other native Anthropic applications" and that Anthropic
  may enforce this without notice. Use this tool at your own risk.
- **The Codex query is a real request.** Codex has no usage endpoint, so each
  query sends one minimal inference request and reads the quota from the
  response headers. That spends a very small amount of quota every time.
  The Claude query costs nothing.
- **Tokens stay put.** They are read from disk on each query, sent only to
  the provider's own server, and never stored, logged, or refreshed. If a
  token has expired, run the official tool once to refresh it.

## Install

You need the [Rust toolchain](https://rustup.rs).

```
cargo install --path crates/agent-usage-cli --locked
```

## Usage

```
agent-usage-cli codex  [--model M] [--effort E] [--auth-file PATH]
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

There is no config file and no built-in polling. To watch usage, wrap the
command in a poller, for example `watch -n 300 agent-usage-cli all`.

### Where credentials are found

| Provider | Order of lookup |
| --- | --- |
| Codex | `--auth-file`, then `CODEX_ACCESS_TOKEN` (with optional `CODEX_ACCOUNT_ID`), then `$CODEX_HOME/auth.json`, then `~/.codex/auth.json` |
| Claude Code | `--credentials-file`, then `$CLAUDE_CONFIG_DIR/.credentials.json`, then `~/.claude/.credentials.json` |

Credentials kept in an OS keychain instead of a file are not supported,
apart from passing a Codex token through `CODEX_ACCESS_TOKEN`.

## JSON output

One provider prints an object; `all` prints an array of them.

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
