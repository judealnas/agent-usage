# Codex usage sources

Status: implemented locally; live Codex comparison and native visual verification
remain manual acceptance checks. The sections below retain the design rationale.

Implementation notes: the additive core API is `sources::fetch` / `sources::raw`
with `codex_account::AccountOptions`. Errors currently use descriptive strings;
`fetched_at` records each attempt and the GUI retains the previous successful
report on transport failure. The App Server transport stays private in
`codex_account.rs`. GUI settings use stable entry IDs and configuration generations.
Issue: [#1](https://github.com/judealnas/agent-usage/issues/1)
Branch: `feat/1-codex-usage-sources`

## Goal and defaults

Support account quota from the installed Codex App Server alongside the existing
inference-header measurement. Keep inference as the default for compatibility.
Account monitoring is opt-in and never falls back to inference automatically.
The two sources may report different limits or even different authenticated
accounts. Do not combine percentages, deduplicate across sources, or label a
quota as exclusive to Astra or another model.

## Core API

Keep the existing `fetch`, `fetch_all`, `CodexOptions`, and `UsageReport` interfaces
working as inference-only compatibility APIs. Add a separate, typed source API
in `agent-usage-core`, used by the updated CLI and GUI:

```rust
enum CodexSource { Account, Inference }
struct CodexAccountOptions {
    executable: PathBuf,       // default: codex; resolve the installed executable
    codex_home: Option<PathBuf>,
    timeout: Duration,         // proposed default: 30 seconds
}
// Conceptual signatures, not final implementation:
fn fetch_codex_source(source: CodexSource, options: &CodexQueryOptions) -> SourceReport;
fn fetch_codex_sources(sources: &[CodexSource], options: &CodexQueryOptions) -> Vec<SourceReport>;
```

`CodexQueryOptions` contains the existing inference options plus account options.
Query selected sources concurrently; return results in a stable requested order.
An individual source failure is a report, not an error that discards other results.

`SourceReport` carries source identity, attempt time, optional success time,
quota groups, and a structured source error. Each `QuotaGroup` carries the
server's limit identifier/name, plan, existing `Metric` windows/balances, optional
enforcement fields, and optional group parsing errors. Inference additionally
records the active-limit label and probe model. Preserve missing enforcement
values as unknown; do not reuse the existing boolean's default `false` to imply
that usage is allowed. Preserve unknown limit-state strings for forward compatibility.

Use `(source, limit_id)` as group identity. For legacy responses without an ID,
use a documented local key such as `legacy`, leaving the server ID absent.
Do not infer identity from the HTTP header prefix. Keep raw response access
separate from normalized reports. A malformed group must not erase valid groups.

## Account adapter and process lifecycle

Add `codex_account.rs` and a small private App Server transport module. Follow
the [official protocol](https://developers.openai.com/codex/app-server/): start
`codex app-server` over stdio, complete `initialize` and `initialized`, then call
`account/rateLimits/read`. No thread or turn is started. Parse every entry in
`rateLimitsByLimitId`; use `rateLimits` only when the map is absent or null. An
empty map is an empty result, not a reason to resurrect the legacy bucket.
Classify windows by their duration, retaining unknown durations.

Initially start one short-lived process per refresh. This keeps CLI and GUI
behavior identical and avoids a background connection manager. A persistent
GUI connection can be evaluated later if measured startup cost warrants it.

The transport must match response IDs, tolerate notifications, reject malformed
messages, handle RPC errors and unexpected EOF, and return an explicit unsupported
response to unexpected server requests. Use one overall deadline covering startup,
handshake, and query. Bound message size and captured stderr, drain both output
streams, and ensure timeout/cancellation interrupts blocking reads. Close stdin,
allow a short bounded shutdown grace period, then terminate and reap the owned
process and join reader tasks on every exit path. Windows launches must be hidden.
Test launcher resolution for native and npm-installed Codex; do not assume a
PowerShell alias or `.cmd` wrapper behaves like a native executable.

Account mode uses the official CLI's login and optional child-only `CODEX_HOME`
override. It does not copy tokens, perform login, or reinterpret the inference
`--auth-file`, `CODEX_ACCESS_TOKEN`, and `CODEX_ACCOUNT_ID` overrides. The UI and
help must clearly identify these separate authentication configurations. Do not
claim account identity matches unless it is actually known.

Enforcement fields mentioned in the issue need verification against the installed
protocol version before mapping: unavailable fields remain unknown. Keep fixtures
and a recorded supported Codex version; unsupported method errors should recommend
updating Codex rather than silently switching sources.

## CLI behavior

```text
agent-usage-cli codex --source account
agent-usage-cli codex --source inference
agent-usage-cli codex --source both --json
agent-usage-cli codex --source account --codex-bin <path> --codex-home <directory>
agent-usage-cli all --codex-source both
```

- Default source remains inference. Existing commands and legacy inference JSON
  retain their output shape; explicit account/both output uses a documented
  `schema_version: 2` envelope with source reports. `all --codex-source account|both`
  uses the same versioned envelope, including the Claude result.
- Reject explicit model/effort/auth-file options in account-only mode; reject
  explicit account process options in inference-only mode. In both mode, each
  option applies only to its own source. Defaults do not trigger conflicts.
- `--raw` inference retains `{status, headers}`. Account raw returns the RPC result
  payload. Both raw returns a source-keyed envelope with per-source result/error;
  it never includes the initialization transcript or credentials.
- Exit 0 when every requested source succeeds, 1 for full or partial query failure,
  and the existing argument-error code for invalid options. Partial output is still
  printed, with each failure clearly associated with its source.

## GUI behavior and migration

### Provider list and editor

Settings retains **General** (including bar colors) and **Providers**. Providers
shows an ordered list of configured entries with a **+** button at the bottom.
Clicking **+** opens an editor with these controls in order:

1. Optional user-defined name.
2. Provider type dropdown: **Codex** or **Claude**. OpenRouter and other providers
   are future extensions and do not appear until implemented.
3. For Codex, a strategy dropdown: **Inference** or **App Server**.
4. Fields for the selected type and strategy:
   - Codex inference: auth.json override, model, and effort.
   - Codex App Server: executable and optional Codex home directory used for login.
   - Claude: credential-file override.
5. Save and Cancel; editing an existing entry also offers Remove.

Selecting another type or strategy changes the visible fields. Only the selected
configuration is validated and saved; hidden draft values cannot affect a query.
Clicking a list entry opens the same editor. Cancel leaves saved settings intact.
A blank or whitespace-only name uses the provider/strategy name, such as
`Codex — Inference`, `Codex — App Server`, or `Claude`. Names are display labels,
not identity or duplicate-detection keys.

This editor contains connection and query settings only. Automatic refresh and
interval controls stay on the usage cards, not in the provider editor.

### Duplicate prevention

Allow the same Codex account once per strategy: one inference entry and one App
Server entry can coexist. Reject a second entry of the same strategy using the
same credential source, even if its name, model, effort, or executable differs.
Likewise reject two Claude entries using the same credential source.

Perform validation in the Rust backend on both creation and editing, excluding
the edited entry itself. Identify the existing entry in the validation message.
Resolve default paths before comparing them: leaving auth.json blank must not
bypass a duplicate check against an explicit path to that same file. Normalize
absolute paths and resolve filesystem aliases where possible, using the platform's
path identity rules. For App Server, compare the effective Codex home. Environment
credential selection is a separate explicit identity case; never store token
values in a duplicate key.

The initial guarantee is duplicate credential-source detection. Distinct files
may contain credentials for the same account; do not claim those are different
accounts or read tokens merely to compare them. Known stable account IDs can
strengthen detection later without changing the one-entry-per-strategy rule.

### Cards, identity, and scheduling

Each configured entry gets its own card, titled with its resolved display name
and identified internally by a stable generated entry ID. Show the provider and
strategy even when a custom name is supplied. Within the card, render each returned
quota group using its actual name/identifier, with last successful update and
error/stale status. Failed refreshes retain previous successful bars but mark them
stale; percentages or expired countdowns must not imply recovery.

Replace provider-only scheduler/cache keys with entry IDs. Include entry ID and
configuration generation in frontend events; group identity within an entry is
the returned limit ID. Keep one in-flight request per entry. Discard late results
after configuration changes or removal. Removing an entry stops its timer and
clears its cached reports. Refreshing an entry cannot invoke another entry or
strategy. Saving General settings or renaming an entry must not query providers;
only changes to connection/query settings invalidate its readings and refresh it.

### Settings migration

Add a settings schema version and an entry list with stable IDs, optional names,
tagged provider configurations, and per-entry card display/refresh settings.
Migrate existing Codex settings into one inference entry and Claude settings into
one Claude entry. Do not automatically add an App Server entry. Preserve visibility,
automatic refresh, intervals, order, credential overrides, pinning, fast-refresh
warning, and gradient settings. Validate migration before saving it. Fast inference
refresh retains the existing warning behavior. Newly added entries use the existing
refresh defaults, adjustable on their cards.

## Delivery and verification

1. Add source report types and account adapter, retaining existing library APIs.
   Test multiple/legacy/empty groups, nullable enforcement, malformed groups,
   notification interleaving, RPC errors, missing login/executable, EOF, oversized
   messages, stderr flooding, deadlines, and process cleanup with a fake server.
2. Add CLI selection, source-specific raw output, versioned JSON, option validation,
   stable ordering, and partial-failure exit tests. Verify account-only fake-server
   transcripts contain no thread or turn calls and no inference HTTP requests.
3. Add the provider list/editor, conditional fields, independent cards and scheduling,
   settings migration, and stale-result tests. Cover create/edit/cancel/remove,
   default names, duplicate credential paths (including default versus explicit),
   same-account cross-strategy entries, and non-duplicate entries. Verify settings
   changes and overlapping refreshes cannot cross entry boundaries or install
   results from an obsolete or removed configuration. Verify refresh intervals
   remain on cards and unsupported provider types are absent from the dropdown.
4. Document prerequisites and API/output compatibility. Manually compare an account
   reading with `/status` under the same login and similar timing. CI uses fixtures
   and fake processes; it never needs live credentials or inference calls.

Out of scope for this first version: direct internal HTTP usage endpoint access,
automatic fallback, model-to-quota classification, and persistent App Server sessions.
