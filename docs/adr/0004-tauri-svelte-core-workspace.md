# Tauri v2 + Svelte, with a Tauri-free core crate

The app is Tauri v2 with a Svelte + TypeScript UI. The Rust code is a Cargo workspace:
- `crates/core` holds the domain, data sources, setup operations and token rules, with no Tauri dependency;
- `crates/cli` is a command-line front end;
- `src-tauri` only binds the core to commands, tray and windows.

The core then builds and tests anywhere, CI and cloud sessions included, without a WebView. Every feature can be tried from the CLI first, including on remote machines. And the UI can ask the core for usage but can never get a token.

## Considered Options

- **C#/WPF:** the most direct route to Windows taskbar docking, but Windows-only.
- **Avalonia:** cross-platform .NET, but less visual freedom than a web UI.
- **Python + PySide6:** heavier to package, and awkward for taskbar docking.

Taskbar docking stays Windows-specific code whatever the stack.
