# agent-usage-gui

A small always-on-top window that shows how much of your coding-agent
subscription limits you have used. Supports Codex (ChatGPT subscription)
and Claude Code (Claude subscription). Windows only for now.

It uses the local `agent-usage-core` Rust library shared with the CLI.
The installer also includes `agent-usage-cli.exe`; using the CLI is optional.

## Read this first

- **Inference and Claude queries use undocumented HTTP endpoints.** Codex's
  App Server strategy uses the documented `account/rateLimits/read` method
  through an installed official Codex CLI.
- **This is outside what the providers document.** Anthropic's Claude Code
  legal page says OAuth authentication is "designed to support ordinary use
  of Claude Code and other native Anthropic applications" and that Anthropic
  may enforce this without notice. Use this app at your own risk.
- **Codex inference refreshes consume quota.** App Server refreshes read account
  limits without starting an inference turn. The sources can measure different
  quota groups; neither is assumed to belong exclusively to a particular model.
- **Authentication depends on strategy.** Inference and Claude read their configured
  credential files. App Server uses the official Codex login and its own credential
  refresh behavior. This monitor does not copy tokens between the two strategies.

## Build and run

Prerequisites:

- [Rust](https://rustup.rs) with the MSVC toolchain, and the Visual Studio
  C++ build tools
- [Node.js](https://nodejs.org) with npm
- The WebView2 runtime (already present on Windows 11)

```
git clone <this repository>
cd agent-usage/apps/agent-usage-gui
npm ci
npm run tauri dev      # run from source
npm run tauri build    # build an installer under the workspace root target/release/bundle
```

Build from the complete monorepo checkout. See the [workspace README](../../README.md) for shared releases.

GUI development and installer builds automatically build and stage the CLI. Before
running Cargo checks directly on a clean checkout, run `npm run prepare:cli` here.
Generated binaries under `src-tauri/binaries/` are ignored by Git.

## Command line interface

In **Settings > General > Command line interface**, enable **agent-usage-cli**.
This adds the GUI installation folder to your Windows user PATH. Open a new
terminal (restart your terminal app if necessary), then run `agent-usage-cli --help`.
The CLI works while the GUI is closed. It uses its own command-line options, not
the GUI's provider configuration.

The default install folder is `%LOCALAPPDATA%\Agent Usage`, containing both
`agent-usage-gui.exe` and `agent-usage-cli.exe`. No administrator access is needed.
General settings save automatically when changed. Save and Cancel are available
only in the provider editor.

Before enabling, the GUI checks the current persisted user and system PATH for
other `agent-usage-cli` executables or command wrappers. It shows conflicts and
blocks enabling until you remove the conflicting installation from PATH yourself.
**Check again** reloads PATH without restarting the GUI. Executables outside PATH,
shell aliases, and shell-specific PATH additions are not detected.

Disabling removes only the exact PATH entry added by this app. Existing PATH
entries and independent CLI installations are left alone. Uninstall removes the
app-owned entry; an enabled preference is retained so reinstalling or upgrading
can restore it, provided no independent CLI conflicts. The standalone CLI remains
available as a separate release download.

## Using the window

**Title bar** (drag it to move the window):

| Button | What it does |
| --- | --- |
| Pin | Toggles always-on-top |
| Refresh | Refreshes every visible or tray-selected entry now |
| + | Adds back a provider you have hidden |
| Sliders | Opens settings |
| – | Minimizes to the taskbar |
| × | Hides to the system tray by default |

### System tray

In **Settings > Providers**, open a provider�s configuration, enable **Show in
system tray**, and click **Save**. Cancel discards changes to these settings. Each selected entry gets its own provider logo surrounded by two rings:
the **outer ring shows 5-hour remaining capacity**, and the **inner ring shows 7-day
remaining capacity**. Each ring changes from green to red using your configured red
threshold. A fully exhausted ring is red; unavailable or expired readings are gray.
Hover for the entry name, percentages, and local reset times.

If a Codex source returns multiple quota groups, choose **Tray quota group** in
that provider�s configuration. Windows from different groups are never combined. A missing selected
group stays unavailable until it returns or you select another group.

Selected entries continue using their refresh settings even when their cards or
the main window are hidden. Manual refresh remains manual. Left-click any tray
icon to restore the window; right-click for Open, Refresh this entry, or Quit.
With no entries selected, an ordinary app icon provides Open and Quit.

**Settings > General > Close to system tray** is enabled by default. Turn it off
to exit when closing the window. **Quit** always exits. This does not enable
starting automatically with Windows.

**Each card** shows one provider: a bar for each rate-limit window, the
percent used, and a countdown to the reset.

- Drag a card by its header to reorder the cards.
- The × in the header hides the card. A hidden provider is not queried.
- The refresh button fetches that provider now.
- **Auto Refresh** turns timed refreshing on or off. When it is on, set the
  interval beside it (a whole number, 1 second or more). Under a minute, the card warns you: fast Codex inference refreshes spend quota, and fast Claude refreshes can get rate-limited.
- If a refresh fails, the card keeps its last values, dims, and shows
  "stale"; hover over that word for the reason.

Drag the window's side edges to change its width. The height fits the content up
to the available screen height, then scrolls. Cards sit side by side when the
window is wide enough.

**Settings** includes:

- **General:** Bar colors, the fast-refresh warning, and CLI integration. Bars shift continuously
  from green at 0% through yellow to red at the configured **Red at (% used)**
  threshold (1–100%, default 90%). They stay red above that threshold. Existing
  red thresholds are retained; the former amber threshold is no longer used.
- **Providers:** A list of configured entries with a **+** button at the bottom.
  Add an optional name, choose Codex or Claude, then configure the connection.
  Codex offers **Inference** (auth.json, model, effort) and **App Server** (Codex
  executable and optional Codex home directory). Only relevant fields are shown.
  Click an entry to edit it; Save, Cancel, and Remove are available. A blank name
  uses the provider and strategy as the card title. Refresh intervals remain on
  the cards.

Each entry has its own card and timer. One Codex credential source can have one
entry per strategy, but duplicate entries within a strategy are rejected even
when names or inference models differ. Default and explicit paths to the same
credential source are treated as duplicates; separate copies of credentials are
not inspected to prove account identity.

App Server requires the official Codex CLI and its login. It inherits `CODEX_HOME`
or uses the directory specified in the entry. Its authentication is separate from
inference auth-file and token overrides. If automatic executable discovery fails,
select the native Codex executable. Account errors never trigger inference probes.
Multiple returned quota groups remain separate on the card.

Existing settings migrate to Codex inference and Claude entries, preserving
credentials, visibility, order, refresh intervals, and General settings. App Server
is added explicitly. Removing an entry does not remove its credentials from disk.

Run `npm test` for color and provider-editor DOM tests. These use mocked Tauri
commands; Rust tests cover credential matching, migration, and fake App Server
processes without live credentials.

The application identifier is `com.judealnas.agentusagegui`. Settings are saved to
`%APPDATA%\agent-usage-gui\settings.json`. On first launch, existing settings and
saved window position/size are copied from the previous locations if the new files
do not exist. Original files are retained, and newer settings are never overwritten.
Credential files and CLI PATH registration are unchanged. The app
rewrites that file whenever something changes, so it is not meant for hand
editing.

## License

MIT
