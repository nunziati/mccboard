# Desktop profiles are launched by mccboard and tagged from outside

A desktop profile other than the default one is the official Claude desktop app, started by mccboard with its own data directory. mccboard creates the folder `%LOCALAPPDATA%\Claude-<label>` first, then runs the packaged `Claude.exe` from the MSIX install folder, found from the package each time because the folder changes at every update. It passes `--user-data-dir=<folder>` and, in the environment, `CLAUDE_CONFIG_DIR=<the account's local instance>`, with Claude's other `CLAUDE*`/`ANTHROPIC*` variables removed. mccboard never modifies, copies or injects code into the app. It sets window and tray properties from its own process, and keeps the profile's taskbar identity while the profile runs:
- every window of the profile gets its own AppUserModelID, the relaunch properties and an icon in its account's color. A window-event hook applies them as soon as Windows creates a window, so a new window never shows under the default app's button;
- a Start-menu shortcut with the same AppUserModelID and icon runs mccboard, which opens the profile, so the profile can be pinned to the taskbar;
- the profile's notification-area icon gets the Claude mark in its account's color and a tooltip naming the account.

The default desktop profile is left alone: it keeps Claude's icon, and its account takes Claude's orange everywhere in mccboard.

Checked by hand on the Windows laptop with Claude 2.42304 (MSIX, package family `Claude_pzs8sxrjxfjjc`; the results are in the desktop profiles spec):
- **Launching.** The exe started this way runs with the package identity. Its data lands in the real folder created beforehand, not in the package's virtualised copy. Two profiles run side by side, and starting an open profile again only brings it to the front.
- **The Code tab.** It uses the `CLAUDE_CONFIG_DIR` the app was started with. The app has no setting for it ("can only be set for all sessions"), so the launch environment is the only way.
- **Taskbar and tray.** Properties set from another process hold, and the app doesn't reset them. The tray icon is addressed by the app's `Electron_NotifyIconHostWindow` window and its icon id.
- **Signing in.** It never relies on the `claude://` link, which the default profile receives. Instead: open the e-mailed link in the browser, cancel opening the app, choose "sign in with a code", and type the code into the profile's window. mccboard guides the user through this.
- **The account.** A profile's account is observed from the names of its `claude-code-sessions\<account uuid>` folders. The token stays in the app's encrypted store and mccboard never reads it (ADR 0002).

The app restarts itself after an update. That restart may lose `--user-data-dir` and does lose `CLAUDE_CONFIG_DIR` (anthropics/claude-code#90868). mccboard detects a profile the app restarted on its own and offers to reopen it, rather than stopping the app's updates. A profile whose Code tab fell back to `~\.claude` would write its account into the primary instance's `.claude.json` (anthropics/claude-code#99240).

## Considered Options

- **multi-claude** (thomasog/multi-claude), the same technique in PowerShell: a young single-author tool. mccboard does the same from its tray process, which also owns accounts and instances.
- **A `claude://` router** that hands sign-in links to the right profile: Windows only lets the user pick it by hand, and it breaks whenever the app changes the link. Signing in with a code needs nothing.
- **Polling windows** every 2 s, as the probe did: a new window shows under the default app's button until the next pass.
- **Disabling the app's auto-update** with its policy, so that restarts never happen: it changes the app's behaviour, and updates belong to the app (nunziati/mccboard#1).
- **A portable copy** of the app outside WindowsApps: it no longer updates itself.
- **The old Squirrel installer** (`%LOCALAPPDATA%\AnthropicClaude`): out of scope. Current installs are MSIX.
