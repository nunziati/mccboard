# The terminal front end is a TUI inside the CLI

A user who works only in a terminal, on a server reached over SSH or on their own machine, has neither the tray nor the Instances window. For them the CLI gets two interactive commands: `mccboard setup`, which guides the setup as the Instances window does, and `mccboard dashboard`, which shows usage and instances as the tray and the window do. Both are a TUI drawn with ratatui on the crossterm backend, in `crates/cli/src/tui/`, and both are front ends like the others. Every step calls `Core`, and the usage they show comes from the view models of the tray and of the taskbar widget. The dashboard refreshes usage at most once a minute, the budget of the tray (ADR 0002, ADR 0003).

The TUI lives in the CLI rather than in a crate of its own, because a server then needs one binary, the one that `install.sh` puts in `~/.local/bin`. On Linux that binary is static (musl) and published on GitHub Releases for each tag. The script downloads it, checks its SHA-256 and installs it without root, in the folder where the official installer of Claude Code puts `claude`.

For the login of Claude Code the TUI leaves the alternate screen and raw mode, runs the login in the current terminal and draws again when it ends. It reads nothing the login prints (ADR 0002). A guard restores the terminal when the TUI ends, fails or panics, because a terminal left in raw mode makes an SSH session unusable.

## Considered Options

- **A TUI crate with a binary of its own** (`crates/tui`): two programs to ship and install, and a second name to learn, for code that calls the core as the CLI does.
- **Questions in the plain CLI**, one prompt after the other with no screen: enough for the setup, but with no live view of usage, which is half of what a terminal user misses.
- **A web UI served by mccboard** on the server and reached through an SSH tunnel: a server process and a port to secure, for one user at a terminal.
- **cursive** instead of ratatui: a widget toolkit with its own event loop, harder to test frame by frame. ratatui renders into a `TestBackend`, and each screen stays plain state.
- **`cargo install` alone**: the Rust toolchain on every server and minutes of compilation for each install.
