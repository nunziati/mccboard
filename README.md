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
# (mccboard-shim.exe) and the CLI (mccboard.exe) and ships them next to the app (ADR 0007); it
# expects the default target/.
TAURI_APP_PATH=../src-tauri npm run tauri build -- --config ../src-tauri/tauri.bundle.conf.json
# Linux packages (.deb, .rpm, AppImage) under target/release/bundle/, without the extra config:
# on Linux the launchers are shell aliases and the CLI is installed on its own.
TAURI_APP_PATH=../src-tauri npm run tauri build
```

On Linux the app has the tray icon and the Instances window. The taskbar widget and the desktop
profiles need Windows, and the window says so in their place. The tray icon shows on panels with a
StatusNotifier tray (KDE Plasma, waybar, GNOME with the AppIndicator extension). AppIndicator has
no tooltip and reports no click on the icon, and for this reason the usage lines are in the menu,
which opens on any click. Starting mccboard again shows the window of the copy already running,
also on a desktop where no tray icon can show. The login of a new instance opens in `$TERMINAL`,
else in `x-terminal-emulator`, else in the first terminal on the PATH among kitty, alacritty,
foot, wezterm, konsole, gnome-terminal, xfce4-terminal and xterm.

On Windows, launchers are copies of `mccboard-shim.exe`, which mccboard looks for next to its own
program: `cargo build --workspace` puts it there for the CLI. A CLI installed on its own needs the
shim installed next to it too: `cargo install --path crates/cli` and `cargo install --path crates/shim`.
The installed app copies its CLI into `~/.mccboard/bin`, next to the launchers, when it starts
and on every launcher sync, so `mccboard` works in any new terminal.

CI (`.github/workflows/ci.yml`) runs the same checks on Ubuntu and Windows and keeps the Windows installers and the Linux packages as artifacts.
