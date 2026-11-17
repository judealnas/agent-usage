# agent-usage-window

A small always-on-top window that shows how much of your coding-agent
subscription limits you have used. Supports Codex (ChatGPT subscription)
and Claude Code (Claude subscription). Windows only for now.

It is a thin window over the `agent-usage` library from [`agent-usage-cli`](https://github.com/judealnas/agent-usage-cli), which is
fetched directly from GitHub by Cargo and also works on its own as a command-line tool.

## Read this first

- **Both providers are queried through undocumented endpoints**, using the
  login tokens the official tools already store on your machine (the Codex
  CLI's `auth.json` and Claude Code's `.credentials.json`). These endpoints
  can change or stop working without notice.
- **This is outside what the providers document.** Anthropic's Claude Code
  legal page says OAuth authentication is "designed to support ordinary use
  of Claude Code and other native Anthropic applications" and that Anthropic
  may enforce this without notice. Use this app at your own risk.
- **Every Codex refresh is a real request.** Codex has no usage endpoint, so
  each refresh sends one minimal inference request and reads the quota from
  the response headers. That spends a very small amount of quota every time,
  so think twice before setting a short Codex refresh interval. Claude
  refreshes cost nothing.
- **Tokens stay put.** They are read from disk on each refresh, sent only to
  the provider's own server, and never stored, logged, or refreshed. If a
  login has expired, run the official tool once to refresh it.

## Build and run

Prerequisites:

- [Rust](https://rustup.rs) with the MSVC toolchain, and the Visual Studio
  C++ build tools
- [Node.js](https://nodejs.org) with npm
- The WebView2 runtime (already present on Windows 11)

```
git clone <this repository>
cd agent-usage-window
npm install
npm run tauri dev      # run from source
npm run tauri build    # build an installer under src-tauri/target/release/bundle
```

Cargo downloads the pinned version of `agent-usage` automatically on the first build.

## Using the window

**Title bar** (drag it to move the window):

| Button | What it does |
| --- | --- |
| Pin | Toggles always-on-top |
| Refresh | Refreshes every card now |
| + | Adds back a provider you have hidden |
| Sliders | Opens settings |
| – | Minimizes to the taskbar |
| × | Closes the app |

**Each card** shows one provider: a bar for each rate-limit window, the
percent used, and a countdown to the reset.

- Drag a card by its header to reorder the cards.
- The × in the header hides the card. A hidden provider is not queried.
- The refresh button fetches that provider now.
- **Auto Refresh** turns timed refreshing on or off. When it is on, set the
  interval beside it (a whole number, 1 second or more). Under a minute, the card warns you: fast Codex refreshes spend quota, and fast Claude refreshes can get rate-limited.
- If a refresh fails, the card keeps its last values, dims, and shows
  "stale"; hover over that word for the reason.

Drag the window's side edges to change its width. The height always fits
the content, and cards sit side by side when the window is wide enough.

**Settings** holds the Codex model and effort used for the probe request,
optional paths to the two credential files, the percentages at which bars
turn amber and red, and a switch for the fast-refresh warning.

Settings are saved to `%APPDATA%\agent-usage-window\settings.json`. The app
rewrites that file whenever something changes, so it is not meant for hand
editing.

## License

MIT
