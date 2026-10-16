//! Desktop profiles' notification-area icons on Windows (spec #34, ticket #38): Claude's mark
//! in the account's color and the tooltip `Claude <label>`, for every running profile but the
//! default one. What to show and when is the core's ([`crate::profile_tray_icons`],
//! [`crate::ProfileTrayKeeper`]); this module applies it.
//!
//! - **Where it runs**: a [`WatcherListener`] on the taskbar watcher's thread
//!   ([`crate::windows_taskbar`]), which looks at the running profiles every few seconds and
//!   tells of Explorer's restarts ([`WatcherEvent::TaskbarCreated`]), then looks again a few
//!   times while the app adds its own icon back.
//! - **The app's icon**: Electron's main process owns a hidden window of class
//!   `Electron_NotifyIconHostWindow`, top-level or message-only, so both are searched. The icon
//!   is modified with `Shell_NotifyIconW(NIM_MODIFY)`, by that window and the icon id, setting
//!   the icon, the tooltip and `NIF_SHOWTIP`. The id is found by trying ids until the shell
//!   accepts one (only the icons of that window can match) and is remembered for the process.
//! - **Icons** (`HICON`s) are loaded once from the cached tray `.ico` at the small-icon size and
//!   kept while a profile uses them; one no longer used is destroyed a look later.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_SHOWTIP, NIF_TIP, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, FindWindowExW, GetClassNameW, GetSystemMetrics,
    GetWindowThreadProcessId, HICON, HWND_MESSAGE, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW,
    SM_CXSMICON, SM_CYSMICON,
};
use windows_sys::core::BOOL;

use crate::desktop_tray::{
    IconAddress, ProfileTrayKeeper, ProfileTrayRepaint, RepaintOutcome, profile_tray_icons,
};
use crate::windows_taskbar::{WatcherEvent, WatcherListener};
use crate::{Language, TOOLTIP_MAX_UTF16};

/// The class of the hidden window that owns an Electron app's tray icons.
const HOST_CLASS: &str = "Electron_NotifyIconHostWindow";

/// The listener that keeps the running profiles' notification-area icons, for
/// [`crate::windows_taskbar::ProfileWatcher::start`]. `language` gives the app's language for
/// the tooltips; it is asked only while a profile runs.
pub fn listener(language: Box<dyn Fn() -> Language + Send>) -> WatcherListener {
    let mut tray = ProfileTray {
        language,
        keeper: ProfileTrayKeeper::new(),
        icons: HashMap::new(),
        retired: Vec::new(),
    };
    Box::new(move |event| tray.on_event(event))
}

struct ProfileTray {
    language: Box<dyn Fn() -> Language + Send>,
    keeper: ProfileTrayKeeper,
    /// Loaded icons by `.ico` file, as `HICON` values (kept as integers: the listener moves to
    /// the watcher's thread).
    icons: HashMap<PathBuf, isize>,
    /// Icons no profile uses any more, destroyed at the next look.
    retired: Vec<isize>,
}

impl ProfileTray {
    fn on_event(&mut self, event: &WatcherEvent<'_>) {
        match event {
            WatcherEvent::Refreshed { profiles, .. } => {
                for icon in self.retired.drain(..) {
                    unsafe { DestroyIcon(icon as HICON) };
                }
                let icons = if profiles.is_empty() {
                    Vec::new()
                } else {
                    profile_tray_icons(profiles, (self.language)())
                };
                let now = Instant::now();
                let due = self.keeper.due(&icons, now);
                if !due.is_empty() {
                    let hosts = host_windows();
                    for repaint in &due {
                        let outcome = self.apply(repaint, &hosts);
                        self.keeper.record(repaint, outcome, Instant::now());
                    }
                }
                let unused: Vec<PathBuf> = self
                    .icons
                    .keys()
                    .filter(|path| !icons.iter().any(|i| i.icon.as_ref() == Some(*path)))
                    .cloned()
                    .collect();
                for path in unused {
                    self.retired.extend(self.icons.remove(&path));
                }
            }
            WatcherEvent::TaskbarCreated => self.keeper.taskbar_created(Instant::now()),
        }
    }

    /// Applies `repaint` to the host window of the first of its processes that has one.
    fn apply(&mut self, repaint: &ProfileTrayRepaint, hosts: &[(isize, u32)]) -> RepaintOutcome {
        let Some(&(hwnd, process)) = repaint
            .icon
            .processes
            .iter()
            .find_map(|pid| hosts.iter().find(|(_, host_pid)| host_pid == pid))
        else {
            return RepaintOutcome::NoHostWindow;
        };
        let icon = repaint.icon.icon.as_deref().and_then(|ico| self.icon(ico));
        for id in repaint.id_candidates(process) {
            if modify(hwnd as HWND, id, icon, &repaint.icon.tooltip) {
                return RepaintOutcome::Applied(IconAddress { process, id });
            }
        }
        RepaintOutcome::NotAccepted
    }

    /// The icon `ico` at the small-icon size, loaded once and kept.
    fn icon(&mut self, ico: &Path) -> Option<HICON> {
        if let Some(&icon) = self.icons.get(ico) {
            return Some(icon as HICON);
        }
        let wide: Vec<u16> = ico
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain([0])
            .collect();
        let icon = unsafe {
            LoadImageW(
                null_mut(),
                wide.as_ptr(),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_LOADFROMFILE,
            )
        } as HICON;
        if icon.is_null() {
            return None;
        }
        self.icons.insert(ico.to_path_buf(), icon as isize);
        Some(icon)
    }
}

impl Drop for ProfileTray {
    fn drop(&mut self) {
        let icons = self.icons.drain().map(|(_, icon)| icon);
        for icon in icons.chain(self.retired.drain(..)) {
            unsafe { DestroyIcon(icon as HICON) };
        }
    }
}

/// Sets icon `id` of `hwnd` to `icon` (when there is one) and `tooltip`; whether the shell
/// accepted it, i.e. whether `hwnd` has an icon with that id.
fn modify(hwnd: HWND, id: u32, icon: Option<HICON>, tooltip: &str) -> bool {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: id,
        uFlags: NIF_TIP | NIF_SHOWTIP,
        ..Default::default()
    };
    if let Some(icon) = icon {
        data.uFlags |= NIF_ICON;
        data.hIcon = icon;
    }
    // NUL-terminated: at most 127 units of the 128.
    for (unit, from) in data
        .szTip
        .iter_mut()
        .zip(tooltip.encode_utf16().take(TOOLTIP_MAX_UTF16))
    {
        *unit = from;
    }
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) != 0 }
}

/// Every Electron tray host window on the desktop with its process: top-level ones, then
/// message-only ones (which `EnumWindows` doesn't list).
fn host_windows() -> Vec<(isize, u32)> {
    let mut found: Vec<(isize, u32)> = Vec::new();
    unsafe {
        EnumWindows(
            Some(each_window),
            &mut found as *mut Vec<(isize, u32)> as LPARAM,
        )
    };
    let class: Vec<u16> = HOST_CLASS.encode_utf16().chain([0]).collect();
    let mut after: HWND = null_mut();
    loop {
        after = unsafe { FindWindowExW(HWND_MESSAGE, after, class.as_ptr(), null_mut()) };
        if after.is_null() {
            break;
        }
        found.push((after as isize, pid_of(after)));
    }
    found
}

unsafe extern "system" fn each_window(hwnd: HWND, found: LPARAM) -> BOOL {
    let found = unsafe { &mut *(found as *mut Vec<(isize, u32)>) };
    if is_host(hwnd) {
        found.push((hwnd as isize, pid_of(hwnd)));
    }
    1
}

fn is_host(hwnd: HWND) -> bool {
    let mut name = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
    length > 0 && String::from_utf16_lossy(&name[..length as usize]) == HOST_CLASS
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}
