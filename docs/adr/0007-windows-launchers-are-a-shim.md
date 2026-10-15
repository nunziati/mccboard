# Windows launchers are a small program, not scripts

On Windows a launcher is a copy of one small program, the shim (`crates/shim`, `mccboard-shim.exe`), saved as `~/.mccboard/bin/claude-b.exe`. Next to it a sidecar, `claude-b.shim`, names the instance. The shim reads the sidecar beside it, sets `CLAUDE_CONFIG_DIR`, finds `claude` on the PATH with `PATHEXT` (skipping its own folder), starts it with the arguments it received, sharing the console, and exits with its exit code. A batch `claude.cmd`, as npm installs it, runs through `cmd.exe /d /c` with Rust's escaping for batch files. Rust refuses an argument cmd.exe can't carry unchanged (a line break) rather than alter it. With an npm install cmd.exe still runs `claude.cmd`, so its Ctrl+C prompt can still appear, as it does for plain `claude`.

Scripts failed in the field (#18). PowerShell picks `claude-b.ps1` over `claude-b.cmd`, and under the *Restricted* policy, the default on Windows client editions, it refuses to run it. cmd.exe can ask "Terminate batch job (Y/N)?" after Ctrl+C, and its quoting mangles `%`, `^`, `&`, `!` and quotes. A program runs in every shell under any policy, and no shell re-quotes its arguments.

The shim is std plus the Windows console API. It knows nothing of mccboard's config. The core writes the sidecar in the format the shim's library defines. A sync replaces the scripts earlier versions wrote (those with mccboard's marker). Until then the doctor reports them as stale, as it does a copy that differs from the shipped shim (after an upgrade) or a sidecar naming another folder. A copy that is running can't be overwritten or deleted, so it is renamed `claude-b.exe.old`, and a later sync deletes it.

The core looks for the shim next to the running program. `cargo build` puts every workspace program in one folder. For the installer, `src-tauri/tauri.bundle.conf.json` builds the shim before the app and adds it as a resource, which Windows installs next to the app. It is a separate config passed to `tauri build --config`: a resource in `tauri.conf.json` would have to exist on every `cargo build` of the app. Without the shim, writing a launcher fails with `launcher_shim_missing`.

## Considered Options

- **Only the `.cmd`:** fixes PowerShell, keeps cmd.exe's Ctrl+C prompt and quoting.
- **A `.ps1` signed or a policy change:** mccboard would be changing the user's security settings.
- **The target folder from `config.toml` instead of a sidecar:** the shim would parse mccboard's config, and break when it changes.
- **The CLI or the app as the shim:** megabytes per launcher, all copied again at every update.
