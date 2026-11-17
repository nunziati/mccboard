# mccboard

**Multi Claude Code dashBoard**: a control center for several Claude Code instances and accounts.

- **Taskbar widget** and **tray icon** showing each account's 5-hour and 7-day usage.
- **Dashboard**: usage trends, exhaustion forecast, per-model limits, active sessions, instance health.
- **Side-by-side Claude desktop profiles**: one per account, each with its own color and taskbar button.
- **Guided setup** of remote machines over SSH: install, login, shared configuration, checks.
- **In a terminal**, on a server or anywhere else: a one-command install, a guided setup and a live dashboard.

Status: in design. Stack: Tauri v2 (Rust + Svelte/TypeScript).

License: MIT.

## Install

mccboard comes in two forms: (1) the desktop app, with the tray icon, the taskbar widget and the windows, and (2) the `mccboard` command, which in a terminal also runs a guided setup and a live dashboard. Both work on the instances of Claude Code and do not replace it. In a terminal, `mccboard setup` installs Claude Code when it is missing, while next to the app you install it with its official installer, `curl -fsSL https://claude.ai/install.sh | bash` on Linux or `irm https://claude.ai/install.ps1 | iex` in PowerShell on Windows.

| Where mccboard runs | What to install | How |
|---|---|---|
| A Linux server, or any Linux terminal (x86_64, aarch64) | The CLI | [One command](#in-a-terminal-on-linux) |
| A Windows desktop | The app, which brings the CLI | [An installer](#the-desktop-app-on-windows) |
| A Linux desktop | The app, and the CLI on its own | [A .deb, .rpm or AppImage](#the-desktop-app-on-linux) |

The CLI also builds [from source](#the-cli-from-source) on Linux and Windows.

### In a terminal on Linux

The install script needs Linux on x86_64 or aarch64 with `curl` or `wget`, `sha256sum` or `shasum`, and `tar`, which most distributions have from the start. It never asks for root and needs no Rust.

1. Run the script:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/nunziati/mccboard/main/install.sh | sh
   ```

   It downloads the `mccboard` CLI of the latest release, checks it against its published SHA-256 and installs it in `~/.local/bin`, where the official installer of Claude Code puts `claude`. Without curl, `wget -qO- https://raw.githubusercontent.com/nunziati/mccboard/main/install.sh | sh` does the same.
2. At a terminal, the script then starts `mccboard setup`, which goes from Claude Code to your accounts and their logins one step at a time ([On a server or in a terminal](#on-a-server-or-in-a-terminal)).
3. When `~/.local/bin` is not on the PATH, the script prints the line to add for bash, zsh or fish. After adding it, open a new terminal and check the install with `mccboard --version`.

Running the same command again updates mccboard. Options go after `sh -s --`, as in `curl -fsSL …/install.sh | sh -s -- --no-setup`, and variables before `sh`, as in `curl -fsSL …/install.sh | MCCBOARD_VERSION=0.2.0 sh`:
- `--no-setup` installs without starting the setup;
- `--uninstall` removes the binary, while `~/.mccboard` and the launchers stay;
- `MCCBOARD_VERSION` installs that version, a pre-release included;
- `MCCBOARD_INSTALL_DIR` installs in another folder;
- `MCCBOARD_DOWNLOAD_URL` downloads from a mirror, together with `MCCBOARD_VERSION`.

The script downloads from GitHub Releases. Pushing a tag `v<version>` equal to the workspace version builds the CLI as static Linux binaries and publishes them with their checksums (`.github/workflows/release.yml`), and a version with a pre-release part, such as `0.2.0-rc.1`, becomes a pre-release, which the script installs only when `MCCBOARD_VERSION` names it.

### The desktop app on Windows

No release holds the app yet, and for now its installers come from CI or from a build of your own.

- **From CI.** Every push to `main` builds them, and the run keeps them as the artifact `mccboard-windows`. Open the latest run on `main` of the [CI workflow](https://github.com/nunziati/mccboard/actions/workflows/ci.yml), download the artifact (GitHub asks you to sign in) and unzip it.
- **From source,** with Rust (stable) and Node.js 22+, in PowerShell:

  ```powershell
  git clone https://github.com/nunziati/mccboard
  cd mccboard\ui
  npm ci
  $env:TAURI_APP_PATH = '..\src-tauri'
  npm run tauri build -- --config ../src-tauri/tauri.bundle.conf.json
  ```

  The installers land in `target\release\bundle\nsis\` and `target\release\bundle\msi\`, at the root of the clone.

Run the setup `.exe` or the `.msi`. Neither installer is signed, and when Windows SmartScreen stops the one downloaded from CI, **More info** and then **Run anyway** let it start. The installer also brings the `mccboard` CLI, which the app copies next to the launchers in `~\.mccboard\bin`. On Windows, linked items need Developer Mode, which mccboard checks for and explains how to turn on ([ADR 0006](docs/adr/0006-windows-developer-mode.md)).

### The desktop app on Linux

The same two ways give the Linux packages: a `.deb`, an `.rpm` and an AppImage. CI keeps them as the artifact `mccboard-linux`, and a build of your own needs Rust (stable), Node.js 22+ and the Tauri system packages, listed for Debian and Ubuntu in [Build and test](#build-and-test):

```sh
git clone https://github.com/nunziati/mccboard
cd mccboard/ui
npm ci
TAURI_APP_PATH=../src-tauri npm run tauri build
```

The packages then sit in `target/release/bundle/`, with the same folders as in the unzipped artifact. From that folder, install the package of your distribution, or run the AppImage, which needs no install:

```sh
sudo apt install ./deb/mccboard_*.deb      # Debian, Ubuntu
sudo dnf install ./rpm/mccboard-*.rpm      # Fedora
chmod +x appimage/mccboard_*.AppImage && ./appimage/mccboard_*.AppImage
```

On Linux the app has the tray icon and the Instances window, while the taskbar widget and the desktop profiles need Windows. The packages hold the app alone, and the `mccboard` command comes from the [install script](#in-a-terminal-on-linux) or from source.

### The CLI from source

On Linux and Windows, with Rust (stable), in a clone of the repository:

```sh
cargo install --locked --path crates/cli
cargo install --locked --path crates/shim    # on Windows only
```

On Windows the launchers are copies of the shim, which mccboard looks for next to its own program ([ADR 0007](docs/adr/0007-windows-launchers-are-a-shim.md)).

## On a server or in a terminal

On a machine without a desktop, such as a server reached over SSH, mccboard runs in the terminal, where `mccboard setup` and `mccboard dashboard` take the place of the app. The CLI comes from the [install script](#in-a-terminal-on-linux).

`mccboard setup` goes through eight steps, each showing what is already done:
1. Claude Code, installed with its official installer once you confirm;
2. the instances to register, or the login of the primary instance where Claude Code has never run;
3. the accounts to add, each with its login in the same terminal;
4. the unknown items of the primary instance, to link or keep private;
5. the drift of each instance, repaired once you confirm;
6. the launchers;
7. the language, and whether Claude Code may renew an expired token;
8. a summary, with Enter opening the dashboard.

The arrows or Tab move between steps, the digits 1 to 8 jump to one, and `q` quits. The setup can stop at any step and run again later.

`mccboard dashboard` shows the session, weekly and model windows of each account with their time to reset, and the login, drift and launcher of each instance. It refreshes usage every minute within the rate limits, as the tray does. Its keys are `r` to refresh, `p` to repair the selected instance, `l` to switch the language, `s` to open the setup and `q` to quit.

Both commands need a terminal. Without one, `mccboard setup` lists the commands that do the same steps, and `mccboard usage` and `mccboard list` show what the dashboard shows.

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
