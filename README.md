# agent-usage

A shared Rust library, command-line tool, and Windows desktop window for viewing
Codex and Claude Code subscription usage. The CLI and GUI are independently
installable; the GUI does not require the CLI executable.

**Disclosure:** The CLI project was generated using Fable 5.2.

## Projects

- [CLI](crates/agent-usage-cli/README.md): `agent-usage-cli`.
- [GUI](apps/agent-usage-window/README.md): Tauri desktop window.
- `crates/agent-usage-core`: shared provider queries, credential lookup, and reports.

Both providers use undocumented endpoints. A Codex query sends a minimal inference
request and consumes quota. See the application READMEs for details.

## Development

Install Rust and, for the GUI, Node.js, Visual Studio C++ build tools, and WebView2.
From the repository root:

```powershell
cargo test
cargo run -p agent-usage-cli -- --help
cargo install --path crates/agent-usage-cli --locked
cd apps/agent-usage-window
npm ci
npm run tauri dev
npm run tauri build -- --ci -- --locked
```

The GUI installer is written under the root `target/release/bundle/nsis/`.
`cargo test` defaults to the core and CLI; on Windows, build the frontend with
`npm run build` in the GUI directory, then run `cargo test --workspace` at the root.

## Shared releases

The initial combined version is **0.1.0**. All Rust packages
inherit the root `[workspace.package].version`. Set that version and the GUI
`package.json` and both root-package version entries in `package-lock.json`
together. Run `python scripts/check-version.py` to verify consistency.

After committing and verifying CI, create and push `vX.Y.Z` for that version.
The release workflow tests both applications and publishes one GitHub Release
with separate Windows CLI and GUI installer downloads, whole-workspace ZIP and
TAR.GZ source archives, and a checksum for every asset. Source builders need the
whole workspace because both applications use local library dependencies.

Releases are published from the shared repository: https://github.com/judealnas/agent-usage.

## Imported history

Both original histories are ancestors of `main`, with their commit hashes
preserved. The CLI's local co-author edits are included. The GUI's pending
changes were captured in the merge commit without modifying its original checkout.
Combined releases use plain `vX.Y.Z` tags. Historical application tags
are not retained in this repository.

## License

MIT. Original project licenses are retained in their respective directories.
