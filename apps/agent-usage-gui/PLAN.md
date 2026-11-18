# agent-usage-gui: original implementation plan

Historical plan: paths and repository names below describe the original `agent-usage-window` project before the monorepo migration and GUI rename.

A small always-on-top desktop window showing subscription usage for coding
agents (Codex and Claude Code now, OpenRouter later). This plan is written
for a session with no access to the conversation that produced it. Every
decision below was agreed with the user; do not reopen them without asking.

## Current state

```
C:\Users\ja384\Home\agent-usage-window\      <- not yet a git repo
  PLAN.md                                    <- this file
  agent-usage\                               <- separate git repo, 1 commit on main, not pushed
    Cargo.toml  Cargo.lock  rust-toolchain.toml  .gitignore
    src\lib.rs  model.rs  config.rs  codex.rs  claude.rs  main.rs
    tests\fixtures\claude_usage.json
```

`agent-usage` is a Rust crate (library `agent_usage` + binary `agent-usage`)
that works today: `cargo run -- all` prints live usage for both providers and
19 unit tests pass. What it contains:

- `model.rs`: `Provider` (`Codex`, `Claude`; `ALL`, `id()`, `display_name()`),
  `Metric` (serde-tagged by `kind`):
  - `Window { label, duration_minutes, used_percent, remaining_percent, resets_at }`
    (`resets_at` is Unix epoch seconds)
  - `Balance { label, used, limit, remaining, currency }`
  - `UsageReport { provider, plan, metrics, limit_reached, details, fetched_at, error }`
  - `Error` (`Credentials`, `Http`, `Parse`, `Config`).
- `codex.rs`: `fetch(&CodexConfig) -> Result<UsageReport, Error>`. Sends one
  minimal inference request to `chatgpt.com/backend-api/codex/responses` and
  reads `x-codex-*` response headers. Each call spends a tiny amount of quota.
- `claude.rs`: `fetch(&ClaudeConfig)` and `fetch_raw(&ClaudeConfig)`. Reads
  `~/.claude/.credentials.json` on every call, never refreshes the token,
  calls `GET api.anthropic.com/api/oauth/usage`, parses defensively. An
  expired token yields "Claude login has expired. Open Claude Code to refresh it."
- `lib.rs`: `fetch(Provider, &Config) -> UsageReport` (never fails; errors go
  in `report.error`) and `fetch_all(&Config) -> Vec<UsageReport>` (concurrent).
- `config.rs`: a TOML config file loader. **This is being removed; see the prologue.**
- `main.rs`: clap CLI with `codex`, `claude [--raw]`, `all`, global `--json`
  and `--config`.

Toolchain on this machine: Rust 1.94 (MSVC), Node 24 + npm, WebView2 runtime,
VS Build Tools 2022 and 2026. No pnpm, dotnet, or go.

## Ground rules

- Windows only for v1, but avoid Windows-only shortcuts that would block
  macOS/Linux later (use `dirs`/Tauri path APIs, not hardcoded paths).
- Commit locally as you go, in both repos. **Do not create GitHub repos or
  push anything** until the user says so. Later they will be public, MIT,
  as `judealnas/agent-usage` and `judealnas/agent-usage-window`.
- Never log or display tokens. Never write to either provider's credential file.
- Do not use pixi. Cargo and npm lockfiles are the dependency management.
- Not in v1: notifications, start-on-login, usage history, tray icon,
  OpenRouter, a release workflow.

## Step 0 (required prologue): make `agent-usage` config-free

This must be finished and committed in the `agent-usage` repo before any GUI
work. The CLI should be self-contained and configured by flags only; polling
is the caller's job.

1. Delete the config file: remove `Config::load`, `Config::default_path`,
   `Config::parse`, the `toml` dependency, the `--config` flag, the
   `Error::Config` variant, `poll_interval_seconds`, and `Thresholds`.
2. Replace `config.rs` with plain option structs (suggested module name
   `options.rs`), each with `Default`:
   - `CodexOptions { model: String, effort: String, auth_file: Option<PathBuf> }`
     with defaults `gpt-6-luna` / `low` / `None`. Keep `CODEX_EFFORTS`.
   - `ClaudeOptions { credentials_file: Option<PathBuf> }`.
   - `Options { codex: CodexOptions, claude: ClaudeOptions }`.
   Derive `Serialize`/`Deserialize` with `#[serde(default)]` so the GUI can
   store them in its own settings file.
3. Library API becomes `fetch(Provider, &Options) -> UsageReport` and
   `fetch_all(&[Provider], &Options) -> Vec<UsageReport>` (concurrent, in
   the given order).
4. CLI:
   - `agent-usage codex [--model M] [--effort E] [--auth-file PATH]`
   - `agent-usage claude [--credentials-file PATH] [--raw]`
   - `agent-usage all [--only codex,claude] [--codex-model M] [--codex-effort E]
     [--codex-auth-file PATH] [--claude-credentials-file PATH]`
     `all` queries every known provider; `--only` picks and orders them.
   - `--json` stays global. Exit code stays non-zero if any report has an error.
5. Update tests: drop the config tests, add a test that `--only` ordering and
   an unknown provider name are handled (parse-level test is enough).

Verify: `cargo test` passes; `cargo run -- all`, `cargo run -- all --only claude --json`,
and `cargo run -- codex --effort low` all work against the live accounts;
`cargo run -- all --config x` is rejected as an unknown flag. Commit.

## Step 1: repository and scaffold

1. `git init -b main` in `agent-usage-window`. Register the existing
   `agent-usage` directory as a submodule using its local path as the URL
   (it will be switched to the GitHub URL on first push).
2. Scaffold Tauri 2 with a vanilla TypeScript + Vite frontend in the repo
   root (`src/` for the frontend, `src-tauri/` for the backend). Use npm.
3. In `src-tauri/Cargo.toml`, depend on the library by path:
   `agent-usage = { path = "../agent-usage" }`.
4. Plugins: `tauri-plugin-single-instance` (second launch focuses the
   existing window) and `tauri-plugin-window-state` (remember size and position).
5. Add `.gitignore` (node_modules, dist, src-tauri/target) and a
   `rust-toolchain.toml` matching the submodule's.

Verify: `npm run tauri dev` opens an empty window; launching it twice leaves
one window. Commit.

## Step 2: settings and backend

Settings live in one app-owned file, `%APPDATA%\agent-usage-window\settings.json`
(resolve via Tauri's app config dir). It is not meant for hand editing. A
missing or unreadable file means defaults. Contents:

- `providers`: ordered list of `{ id, shown, auto_refresh, interval_seconds }`
  for every known provider. Defaults: both shown, auto refresh on, 300 s.
  Providers known to the app but missing from the file are appended as shown.
- `codex`: `{ model, effort, auth_file }`; `claude`: `{ credentials_file }`
  (the library's option structs).
- `thresholds`: `{ warn: 70, critical: 90 }` (percent used).
- `warn_fast_codex_polling`: `true`.
- `pinned`: `true` (always-on-top).

Backend responsibilities (Rust):

- Own one timer per shown provider with auto refresh on. Hidden providers
  and providers with auto refresh off are never polled automatically.
- Fetch with `agent_usage::fetch` on a blocking thread (it uses blocking
  HTTP). Providers are independent: a slow Codex call must not delay Claude.
- Emit each finished `UsageReport` to the frontend as an event.
- At most one fetch in flight per provider. A refresh request for a provider
  that is already fetching is dropped silently.
- A manual refresh restarts that provider's timer.
- Commands: get settings, save settings, refresh one provider, refresh all,
  set a provider's auto refresh and interval, set provider order, show or
  hide a provider, set pinned. Any settings change applies immediately
  (timers restart) and is written to disk; no app restart is ever needed.
- Reject intervals below 1 second.
- On startup, fetch every shown provider once, whatever its auto refresh setting.

Verify: with a temporary debug log of emitted events, reports arrive on
startup and at the configured interval; hiding a provider stops its fetches;
settings survive a restart. Add Rust unit tests for settings defaults,
merging in a newly known provider, and interval validation. Commit.

## Step 3: main view

Window: frameless, always-on-top by default, follows the Windows light/dark
setting. Plain look: system font, rounded cards, thin progress bars, no
logos or provider brand colors. Disable the default browser right-click menu.

Title bar (custom, drag it to move the window), buttons in this order:
pin toggle, refresh all, "+", gear, minimize (to taskbar), close.

Sizing: width user-resizable with a minimum of about 260 px; height always
fits the content (resize the window from the frontend when content height
changes). When the window is wide enough for two cards side by side, cards
flow into columns. Size, position, and pin state persist.

Each card:

- **Header**: provider display name and plan (for example "Codex · edu_plus"),
  and an "X" button that hides the card. Dragging the header reorders cards:
  other cards shift to show the landing spot, and the new order is saved on
  drop. The window itself moves only by the title bar.
- **Body**: for each `Window` metric, a label, a thin progress bar, percent
  used, and a countdown ("resets in 2h 14m"). Bar color by percent used:
  below `warn` green, from `warn` amber, from `critical` red. For each
  `Balance` metric, show the amounts as text (used of limit, remaining,
  currency). Render metrics generically from the report, so Claude's
  model-scoped weekly window appears whenever the library returns it.
- **Footer**: a refresh button, an "Auto Refresh" switch, and, only while the
  switch is on, a number field with a unit selector (seconds, minutes,
  hours). The interval applies on Enter or blur. Values under 1 second are
  rejected. For Codex, an interval under 60 seconds shows a warning that
  every poll is a real request against quota; it warns and does not block,
  and is suppressed when `warn_fast_codex_polling` is off.
- **Status**: "updated 3 min ago", or "stale".

Behavior:

- The refresh button is always enabled. Clicking it while that provider is
  already fetching does nothing. There is no spinner; the "updated" text
  changing is the only sign a fetch finished.
- Countdowns and "updated … ago" tick in the frontend once a minute from
  stored timestamps, without re-fetching. (When an interval is under a
  minute, tick often enough that the display is not misleading.)
- Fetch fails after an earlier success: keep the last values, dim the card,
  show a "stale" marker with the error text on hover.
- Fetch fails with no earlier success: show the error message in the card body.
- A window whose reset time has passed since the last fetch shows 0% and
  "reset, awaiting refresh" until the next successful fetch.
- "+" opens a small dropdown listing providers not currently shown. Picking
  one adds its card at the bottom and fetches it immediately. The button is
  greyed out when nothing is hidden. With every provider hidden, the window
  shows "No providers shown" pointing at the "+".

Verify by running the app: both cards show live numbers matching
`agent-usage all`; hide and re-add a card; drag to reorder and restart to
confirm the order held; toggle auto refresh and change the interval; set a
Codex interval of 30 s and see the warning; point the Claude credentials
path at a missing file in settings (step 4) to see both error states; resize
across the two-column breakpoint; toggle pin; check light and dark themes.
Commit.

## Step 4: settings view

The gear swaps the card list for a settings view in the same window (not a
separate window). Fields:

- Codex model (text) and effort (select from the library's `CODEX_EFFORTS`).
- Codex auth file path and Claude credentials file path (text, empty means
  the default lookup).
- Warn and critical thresholds (numbers, 0–100, warn below critical).
- "Warn when Codex polls faster than once a minute" toggle.

Save and Cancel buttons; both return to the cards. Save validates, applies
everything at once, writes the file, and restarts polling without an app
restart. Provider visibility is not in this view: the "X" and "+" are the
only way to hide and show.

Verify: change the Codex model to a nonsense value and confirm the card shows
the server's error, then restore it; change thresholds and see bar colors
change; Cancel discards edits. Commit.

## Step 5: READMEs

One in each repo. Both must state plainly that:

- Both providers are queried through **undocumented endpoints** using the
  user's existing login tokens (Codex CLI's `auth.json`, Claude Code's
  `.credentials.json`). They can break without notice.
- This use is outside what the providers document. Anthropic's Claude Code
  legal page says OAuth authentication is "designed to support ordinary use
  of Claude Code and other native Anthropic applications" and reserves the
  right to enforce that. Users run this at their own risk.
- The Codex query sends a real minimal inference request and spends a very
  small amount of quota on every poll.
- Tokens are read from disk on each query, sent only to the provider's own
  server, and never stored, logged, or refreshed.

`agent-usage` README: install (`cargo install --path .`, and later
`cargo install --git`), commands and flags, the `--json` shape, and using an
external poller such as `watch`. `agent-usage-window` README: what it is,
prerequisites (Rust, Node, WebView2, MSVC build tools), cloning with
`--recurse-submodules`, `npm install`, `npm run tauri dev` / `build`, and a
short tour of the controls. Add an MIT `LICENSE` to both. Commit.

## Later (not part of this plan)

- GitHub repos and first push (ask the user first), then switch the
  submodule URL to GitHub.
- A GitHub Actions workflow building a Windows installer for the window and
  an exe for the CLI on tagged releases.
- OpenRouter as a third provider, using the `Balance` metric.
