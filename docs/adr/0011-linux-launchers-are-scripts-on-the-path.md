# Linux launchers are scripts on the PATH

On Linux every launcher is a small `sh` script in `~/.local/bin` (for example `~/.local/bin/claude-b`). The script exports `CLAUDE_CONFIG_DIR` and runs `exec claude "$@"`. The official installer of Claude Code puts `claude` in the same folder. `install.sh` puts `mccboard` there too. Most users of Claude Code already have this folder on their PATH for that reason. A second line in each script marks it as mccboard's. Only the scripts with that marker are read, rewritten or removed by mccboard. A file of the user's with the name of a launcher is never overwritten: writing that launcher fails with `launcher_name_taken` instead. The doctor then reports the launcher as stale.

Up to 0.1 a launcher was an alias in a block of `~/.bashrc` (`~/.zshrc` for zsh users). Aliases failed in the field on a machine whose login shell is bash. Because its `~/.bashrc` ran `exec fish` before the block, fish never read the alias. `claude-b` was then an unknown command. The same happens with fish, nushell or any other shell that a user starts from bash or chooses as the login shell. It also happens with every program that runs commands without an interactive shell, such as an editor, a cron job or `ssh host claude-b`. A script on the PATH works in all of them and needs no `source` after a sync.

Every sync on Linux removes the alias block of earlier versions from `~/.bashrc` and `~/.zshrc`. The rest of each file stays byte for byte as it was. A file that is a link is edited where it points. A block without its end marker stays as it is. Since the scripts work without the cleanup, a file that cannot be read or written stays as it is too.

## Considered Options

- **Aliases for every shell (bash, zsh, fish…):** one rc format per shell, still blind to the shells mccboard does not know and to the runs without an interactive shell.
- **Scripts in `~/.mccboard/bin`:** one more folder for the user to put on the PATH, when `~/.local/bin` is already there for `claude`.
- **Symbolic links to one program:** a link cannot carry the folder of the instance. Like the Windows shim (ADR 0007), it would need a sidecar for no gain over a script on Linux.
