# mccboard

**Multi Claude Code dashBoard**: a control center for several Claude Code instances and accounts.

- **Taskbar widget** and **tray icon** showing each account's 5-hour and 7-day usage.
- **Dashboard**: usage trends, exhaustion forecast, per-model limits, active sessions, instance health.
- **Side-by-side Claude desktop profiles**: one per account, each with its own color and taskbar button.
- **Guided setup** of remote machines over SSH: install, login, shared configuration, checks.

Status: in design. Stack: Tauri v2 (Rust + Svelte/TypeScript).

License: MIT.

## Build and test

Needs Rust (stable), Node.js 22+, and on Linux the Tauri system packages
(`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`).

```sh
# Rust workspace: core, CLI (`mccboard`), launcher shim, Tauri app
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p mccboard-cli -- scan          # or: scan --json
cargo run -p mccboard-cli -- register ~/.claude   # then: list, unregister, account set

# UI (Svelte + TypeScript)
cd ui
npm ci
npm run check                              # type-check
npm run build

# The app (run from ui/; the Tauri CLI needs to be told where src-tauri is)
TAURI_APP_PATH=../src-tauri npm run tauri dev     # PowerShell: $env:TAURI_APP_PATH='..\src-tauri'
# Windows installer under target/release/bundle/. The extra config builds the launcher shim
# (mccboard-shim.exe) and ships it next to the app (ADR 0007); it expects the default target/.
TAURI_APP_PATH=../src-tauri npm run tauri build -- --config ../src-tauri/tauri.bundle.conf.json
```

On Windows, launchers are copies of `mccboard-shim.exe`, which mccboard looks for next to its own
program: `cargo build --workspace` puts it there for the CLI. A CLI installed on its own needs the
shim installed next to it too: `cargo install --path crates/cli` and `cargo install --path crates/shim`.

CI (`.github/workflows/ci.yml`) runs the same checks on Ubuntu and Windows and keeps the Windows installer as an artifact.
