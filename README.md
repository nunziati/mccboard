# mccboard

**Multi Claude Code dashBoard**: a control center for several Claude Code instances and accounts.

- **Taskbar widget** and **tray icon** showing each account's 5-hour and 7-day usage.
- **Dashboard**: usage trends, exhaustion forecast, per-model limits, active sessions, instance health.
- **Side-by-side Claude desktop profiles**: one per account, each with its own color and taskbar button.
- **Guided setup** of remote machines over SSH: install, login, shared configuration, checks.
- **In a terminal**, on a server or anywhere else: a one-command install, a guided setup and a live dashboard.

Status: in design. Stack: Tauri v2 (Rust + Svelte/TypeScript).

License: MIT.

## On a server or in a terminal

On a machine without a desktop, such as a server reached over SSH, mccboard runs in the terminal. One command installs it:

```sh
curl -fsSL https://raw.githubusercontent.com/nunziati/mccboard/main/install.sh | sh
```

The script works on Linux, on x86_64 and aarch64. It downloads the `mccboard` CLI of the latest release, checks it against its published SHA-256 and installs it in `~/.local/bin`, where the official installer of Claude Code puts `claude`. At a terminal it then starts `mccboard setup`. It never asks for root, and when `~/.local/bin` is not on the PATH it prints the line to add to your shell. Running the same command again updates mccboard.

Options go after `sh -s --`, as in `curl -fsSL …/install.sh | sh -s -- --no-setup`, and variables before `sh`, as in `curl -fsSL …/install.sh | MCCBOARD_VERSION=0.2.0 sh`:
- `--no-setup` installs without starting the setup;
- `--uninstall` removes the binary, while `~/.mccboard` and the launchers stay;
- `MCCBOARD_VERSION` installs that version, a pre-release included;
- `MCCBOARD_INSTALL_DIR` installs in another folder;
- `MCCBOARD_DOWNLOAD_URL` downloads from a mirror, together with `MCCBOARD_VERSION`.

`mccboard setup` goes through eight steps, each showing what is already done:
1. Claude Code, installed with its official installer once you confirm;
2. the instances to register;
3. the accounts to add, each with its login in the same terminal;
4. the unknown items of the primary instance, to link or keep private;
5. the drift of each instance, repaired once you confirm;
6. the launchers;
7. the language, and whether Claude Code may renew an expired token;
8. a summary, with Enter opening the dashboard.

The arrows or Tab move between steps, the digits 1 to 8 jump to one, and `q` quits. The setup can stop at any step and run again later.

`mccboard dashboard` shows the session, weekly and model windows of each account with their time to reset, and the login, drift and launcher of each instance. It refreshes usage every minute within the rate limits, as the tray does. Its keys are `r` to refresh, `p` to repair the selected instance, `l` to switch the language, `s` to open the setup and `q` to quit.

Both commands need a terminal. Without one, `mccboard setup` lists the commands that do the same steps, and `mccboard usage` and `mccboard list` show what the dashboard shows.

The script downloads from GitHub Releases. Pushing a tag `v<version>` equal to the workspace version builds the CLI as static Linux binaries and publishes them with their checksums (`.github/workflows/release.yml`), and a version with a pre-release part, such as `0.2.0-rc.1`, becomes a pre-release, which the script installs only when `MCCBOARD_VERSION` names it.

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
```

On Windows, launchers are copies of `mccboard-shim.exe`, which mccboard looks for next to its own
program: `cargo build --workspace` puts it there for the CLI. A CLI installed on its own needs the
shim installed next to it too: `cargo install --path crates/cli` and `cargo install --path crates/shim`.
The installed app copies its CLI into `~/.mccboard/bin`, next to the launchers, when it starts
and on every launcher sync, so `mccboard` works in any new terminal.

CI (`.github/workflows/ci.yml`) runs the same checks on Ubuntu and Windows and keeps the Windows installer as an artifact.
