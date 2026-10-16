# The taskbar widget is a native window docked into the taskbar

The Windows 11 taskbar has no place for third-party content: desk bands are gone, and the notification area holds only icons. Every tool that shows something on it does the same thing, and mccboard does it too. The taskbar widget is a plain Win32 window, made a child of the taskbar (`Shell_TrayWnd`) and placed just left of the notification area (`TrayNotifyWnd`). It runs on its own thread inside mccboard's process and is drawn natively. The core computes what it shows (each account's windows, pace, time to reset and colors); the Windows module only docks, draws and forwards clicks. The tray and the dashboard stay as they are.

The window reserves no space on the taskbar, so mccboard keeps the rules narrow:
- It docks only on the primary taskbar, and only when that taskbar is horizontal at the bottom of the screen.
- When it would cover app buttons, it is hidden until there is room again. App buttons are found with UI Automation.
- Whenever the widget is not shown, the tray icon carries the worst weekly window instead of mccboard's logo, so the reading is never lost.
- It never calls `SHAppBarMessage`. That call is a synchronous send to Explorer, and in other tools it deadlocked with it. The taskbar's rectangles come from `GetWindowRect`.
- A restart of Explorer destroys the child window. mccboard notices and docks a new one.

How it is done follows CodeZeno/Claude-Code-Usage-Monitor (commit d0ab8b7), the tool used before mccboard: `SetParent` with `WS_CHILD`, the reset of `WS_EX_LAYERED` that Windows 11 needs after reparenting, `UpdateLayeredWindow` drawing, the WinEvent hook on the taskbar's thread, and the watchdog.

## Considered Options

- **A Tauri (WebView2) window reparented into the taskbar.** Some Tauri apps do this. It costs a whole WebView2 process tree for a strip about 300×40 px. Its transparency fights with Explorer's composition, and the reparenting only takes after the page has loaded.
- **A floating window above the taskbar**, which is what CodeZeno falls back to when the widget would cover app buttons. It covers windows, it has to dodge full-screen apps, and it is no longer "on the taskbar".
- **A desk band.** Windows 11's taskbar doesn't host them.
- **Every taskbar, vertical and top taskbars included.** Each layout breaks differently with each Windows update. Left out until someone needs them.
