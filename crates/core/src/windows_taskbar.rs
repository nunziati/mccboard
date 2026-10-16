//! The running desktop profiles watcher (Windows, spec #34, ticket #37): one thread in
//! mccboard's app that gives every window of a running desktop profile the profile's own
//! taskbar identity, the moment the window is created.
//!
//! - **What a window gets** ([`crate::WindowIdentity`], from
//!   [`crate::Core::running_desktop_profiles`]): through the window's property store, the
//!   relaunch command, display name and icon, then the AppUserModelID; and `WM_SETICON`, small
//!   and big, at the window's DPI. The default desktop profile's windows are never touched.
//! - **When**: a window-event hook (`SetWinEventHook`, `EVENT_OBJECT_CREATE` to
//!   `EVENT_OBJECT_SHOW`, out of context) sees every top-level window created or shown on the
//!   desktop. A window is tagged at creation, before the taskbar gives it a button, so a chat
//!   opened in a new window never shows under the default profile's button first. A window of
//!   a process the watcher doesn't know yet (a profile just started) makes it ask the core
//!   again on the spot, when the process is the desktop app's `Claude.exe`.
//! - **Sweeps**: when the watcher starts (profiles already open), every few seconds, and on
//!   [`ProfileWatcher::refresh`] (the app calls it when `config.toml` changes or a profile
//!   opens), it asks the core for the running profiles and tags the windows whose identity
//!   changed: a new label or color reaches open windows too.
//! - **Icons** (`HICON`s) are loaded from the cached `.ico` files and kept alive while windows
//!   may use them: `WM_SETICON` doesn't copy them. One replaced by a new color is destroyed a
//!   sweep later.
//!
//! **Seam for more consumers** (the tray's per-profile icons, #38): the watcher's thread has a
//! message loop and is the one place that knows which profiles run. A consumer is a
//! [`WatcherListener`] passed to [`ProfileWatcher::start`]; it runs on this thread after every
//! sweep with [`WatcherEvent::Refreshed`] (also when nothing changed, so it doubles as a
//! periodic repaint). An event the thread should react to (Explorer's `TaskbarCreated`, which
//! needs a hidden top-level window: create it in `run` before the loop, and in its window
//! procedure queue it like [`Pending::Refresh`]) becomes a new [`WatcherEvent`] variant; the
//! enum is non-exhaustive for that.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::mpsc;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DestroyIcon, DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_SHOW, EnumWindows, GA_PARENT, GW_OWNER, GetAncestor, GetDesktopWindow,
    GetMessageW, GetWindow, GetWindowThreadProcessId, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON,
    IsWindowVisible, KillTimer, LR_LOADFROMFILE, LoadImageW, MSG, OBJID_WINDOW, PM_NOREMOVE,
    PeekMessageW, PostMessageW, PostThreadMessageW, SM_CXICON, SM_CXSMICON, SetTimer,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_QUIT, WM_SETICON,
    WM_TIMER, WM_USER,
};
use windows_sys::core::BOOL;

use crate::desktop_identity::{RunningProfile, WindowIdentity};
use crate::windows_shell::{self, Com};

/// How often the watcher sweeps without being asked.
const SWEEP_EVERY_MS: u32 = 5_000;

/// Posted to the watcher's thread by [`ProfileWatcher::refresh`].
const WM_REFRESH: u32 = WM_APP + 1;

/// The running desktop profiles, from the core: [`crate::Core::running_desktop_profiles`].
pub type ProfileSource = Box<dyn Fn() -> Vec<RunningProfile> + Send>;

/// A consumer of the watcher's events, called on the watcher's thread.
pub type WatcherListener = Box<dyn FnMut(&WatcherEvent<'_>) + Send>;

/// What the watcher tells its listeners.
#[derive(Debug)]
#[non_exhaustive]
pub enum WatcherEvent<'a> {
    /// The watcher asked the core which profiles run (every few seconds, when asked to, and
    /// when a profile's window appeared): `profiles` is the answer, `changed` whether it
    /// differs from the previous one.
    Refreshed {
        profiles: &'a [RunningProfile],
        changed: bool,
    },
}

/// The watcher's thread. It runs until this is dropped.
#[derive(Debug)]
pub struct ProfileWatcher {
    thread_id: u32,
}

impl ProfileWatcher {
    /// Starts the watcher's thread: it hooks window events, sweeps the windows already open,
    /// then keeps watching. `source` is called on that thread whenever the running profiles
    /// may have changed; `listeners` after each call.
    pub fn start(
        source: ProfileSource,
        listeners: Vec<WatcherListener>,
    ) -> io::Result<ProfileWatcher> {
        let (ready, started) = mpsc::channel();
        std::thread::Builder::new()
            .name("mccboard-taskbar".into())
            .spawn(move || run(source, listeners, ready))?;
        let thread_id = started
            .recv()
            .map_err(|_| io::Error::other("the taskbar watcher's thread stopped"))?;
        Ok(ProfileWatcher { thread_id })
    }

    /// Asks the watcher to sweep now: after `config.toml` changed or a profile was opened.
    pub fn refresh(&self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_REFRESH, 0, 0) };
    }
}

impl Drop for ProfileWatcher {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
    }
}

/// Work for the watcher's thread, done one item at a time: a window event can arrive while
/// another is handled (COM and window calls dispatch messages), and then waits its turn.
enum Pending {
    Event { event: u32, hwnd: isize },
    Refresh,
}

/// What the watcher's thread knows. Borrowed only for plain Rust work, never across a call
/// that may dispatch messages.
struct State {
    source: Option<ProfileSource>,
    listeners: Vec<WatcherListener>,
    profiles: Vec<RunningProfile>,
    /// Pid → index in `profiles`.
    by_pid: HashMap<u32, usize>,
    /// Pids known not to run a profile, until the next sweep.
    not_profile: HashSet<u32>,
    /// Window → the identity it was tagged with ([`identity_key`]).
    tagged: HashMap<isize, String>,
    /// Loaded icons by file and size.
    icons: HashMap<(PathBuf, i32), HICON>,
    /// Icons no profile uses any more, destroyed at the next sweep.
    retired: Vec<HICON>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static PENDING: RefCell<VecDeque<Pending>> = const { RefCell::new(VecDeque::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn run(source: ProfileSource, listeners: Vec<WatcherListener>, ready: mpsc::Sender<u32>) {
    let _com = Com::init();
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(State {
            source: Some(source),
            listeners,
            profiles: Vec::new(),
            by_pid: HashMap::new(),
            not_profile: HashSet::new(),
            tagged: HashMap::new(),
            icons: HashMap::new(),
            retired: Vec::new(),
        })
    });
    unsafe {
        // Creates the thread's message queue, so a refresh posted from now on isn't lost.
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let _ = ready.send(GetCurrentThreadId());

        // The hook first, then the sweep: a window created meanwhile waits in the queue.
        let hook = SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_SHOW, // EVENT_OBJECT_DESTROY lies in between
            null_mut(),
            Some(on_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        queue(Pending::Refresh);
        let timer = SetTimer(null_mut(), 0, SWEEP_EVERY_MS, None);

        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.hwnd.is_null() && (msg.message == WM_REFRESH || msg.message == WM_TIMER) {
                queue(Pending::Refresh);
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        KillTimer(null_mut(), timer);
        if !hook.is_null() {
            UnhookWinEvent(hook);
        }
        let icons = with_state(|s| {
            let mut all: Vec<HICON> = s.icons.drain().map(|(_, icon)| icon).collect();
            all.append(&mut s.retired);
            all
        });
        for icon in icons.into_iter().flatten() {
            DestroyIcon(icon);
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_null() || id_object != OBJID_WINDOW || id_child != CHILDID_SELF as i32 {
        return;
    }
    queue(Pending::Event {
        event,
        hwnd: hwnd as isize,
    });
}

/// Queues `item` and, unless the thread is busy with another item already (this is a
/// re-entrant call), handles everything queued.
fn queue(item: Pending) {
    PENDING.with(|pending| pending.borrow_mut().push_back(item));
    if BUSY.replace(true) {
        return;
    }
    while let Some(item) = PENDING.with(|pending| pending.borrow_mut().pop_front()) {
        match item {
            Pending::Event { event, hwnd } => on_window(event, hwnd as HWND),
            Pending::Refresh => sweep(),
        }
    }
    BUSY.set(false);
}

fn on_window(event: u32, hwnd: HWND) {
    if event == EVENT_OBJECT_DESTROY {
        with_state(|s| s.tagged.remove(&(hwnd as isize)));
        return;
    }
    if event != EVENT_OBJECT_CREATE && event != EVENT_OBJECT_SHOW {
        return;
    }
    if !is_main_window(hwnd) {
        return;
    }
    // Shown: tagged at creation already, but the app may have set its own icon since.
    if event == EVENT_OBJECT_SHOW && unsafe { IsWindowVisible(hwnd) } == 0 {
        return;
    }
    let pid = pid_of(hwnd);
    if let Some(identity) = identity_for(pid) {
        apply(hwnd, &identity, event == EVENT_OBJECT_SHOW);
    }
}

/// A top-level window without an owner: one that can have a taskbar button. Message-only
/// windows aren't top-level.
fn is_main_window(hwnd: HWND) -> bool {
    unsafe {
        GetAncestor(hwnd, GA_PARENT) == GetDesktopWindow() && GetWindow(hwnd, GW_OWNER).is_null()
    }
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

enum Lookup {
    Profile(WindowIdentity),
    NotProfile,
    Unknown,
}

fn lookup(pid: u32) -> Lookup {
    with_state(|s| match s.by_pid.get(&pid) {
        Some(&index) => Lookup::Profile(s.profiles[index].identity.clone()),
        None if s.not_profile.contains(&pid) => Lookup::NotProfile,
        None => Lookup::Unknown,
    })
    .unwrap_or(Lookup::NotProfile)
}

/// The identity of the windows of process `pid`, asking the core again for a desktop app
/// process the watcher doesn't know yet (a profile just started).
fn identity_for(pid: u32) -> Option<WindowIdentity> {
    match lookup(pid) {
        Lookup::Profile(identity) => return Some(identity),
        Lookup::NotProfile => return None,
        Lookup::Unknown => {}
    }
    if is_desktop_app_program(pid) {
        update_profiles();
        if let Lookup::Profile(identity) = lookup(pid) {
            return Some(identity);
        }
    }
    with_state(|s| s.not_profile.insert(pid));
    None
}

/// Whether process `pid` runs a program named `Claude.exe`: the desktop app, whichever
/// profile (or Claude Code's `claude.exe`, which the core then tells apart).
fn is_desktop_app_program(pid: u32) -> bool {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let read = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        );
        CloseHandle(process);
        if read == 0 {
            return false;
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or_default();
        name.eq_ignore_ascii_case("claude.exe")
    }
}

/// Asks the core which profiles run and tells the listeners. Neither the source nor the
/// listeners run under the state's borrow.
fn update_profiles() {
    let Some(Some(source)) = with_state(|s| s.source.take()) else {
        return;
    };
    let profiles = source();
    let listeners = with_state(|s| {
        s.source = Some(source);
        let changed = s.profiles != profiles;
        s.by_pid = profiles
            .iter()
            .enumerate()
            .flat_map(|(index, p)| p.pids.iter().map(move |&pid| (pid, index)))
            .collect();
        s.not_profile.clear();
        s.profiles = profiles;
        (std::mem::take(&mut s.listeners), changed)
    });
    let Some((mut listeners, changed)) = listeners else {
        return;
    };
    let profiles = with_state(|s| s.profiles.clone()).unwrap_or_default();
    for listener in &mut listeners {
        listener(&WatcherEvent::Refreshed {
            profiles: &profiles,
            changed,
        });
    }
    with_state(|s| {
        listeners.append(&mut s.listeners);
        s.listeners = listeners;
    });
}

/// Destroys the icons retired by the previous sweep, asks the core which profiles run, tags
/// every window of theirs whose identity changed, then retires the icons no profile uses.
fn sweep() {
    for icon in with_state(|s| std::mem::take(&mut s.retired))
        .into_iter()
        .flatten()
    {
        unsafe { DestroyIcon(icon) };
    }
    update_profiles();

    let mut windows: Vec<isize> = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            &mut windows as *mut Vec<isize> as LPARAM,
        )
    };
    for hwnd in windows {
        let hwnd = hwnd as HWND;
        if !is_main_window(hwnd) {
            continue;
        }
        if let Lookup::Profile(identity) = lookup(pid_of(hwnd)) {
            apply(hwnd, &identity, false);
        }
    }

    with_state(|s| {
        let in_use: HashSet<&Path> = s
            .profiles
            .iter()
            .filter_map(|p| p.identity.icon.as_deref())
            .collect();
        let unused: Vec<(PathBuf, i32)> = s
            .icons
            .keys()
            .filter(|(path, _)| !in_use.contains(path.as_path()))
            .cloned()
            .collect();
        for key in unused {
            if let Some(icon) = s.icons.remove(&key) {
                s.retired.push(icon);
            }
        }
    });
}

unsafe extern "system" fn collect_window(hwnd: HWND, windows: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(windows as *mut Vec<isize>) };
    windows.push(hwnd as isize);
    1
}

/// What a window was tagged with: tagging again with the same is skipped.
fn identity_key(identity: &WindowIdentity) -> String {
    format!(
        "{}|{}|{}|{}",
        identity.app_user_model_id,
        identity.display_name,
        identity.relaunch_command.as_deref().unwrap_or_default(),
        identity
            .icon
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    )
}

/// Tags `hwnd` with `identity` unless it has it already, and sets its icons when it is newly
/// tagged or `icons_again`.
fn apply(hwnd: HWND, identity: &WindowIdentity, icons_again: bool) {
    let key = identity_key(identity);
    let tagged = with_state(|s| s.tagged.get(&(hwnd as isize)) == Some(&key)).unwrap_or(false);
    if !tagged {
        // Fails for a window gone meanwhile; its next event or sweep tries again.
        if unsafe { windows_shell::tag_window(hwnd, identity) }.is_ok() {
            with_state(|s| s.tagged.insert(hwnd as isize, key));
        }
    }
    if (!tagged || icons_again)
        && let Some(icon) = &identity.icon
    {
        set_icons(hwnd, icon);
    }
}

/// Sends `WM_SETICON` (small and big) with the frames of `ico` that fit the window's DPI.
fn set_icons(hwnd: HWND, ico: &Path) {
    let dpi = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    };
    for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
        let size = unsafe { GetSystemMetricsForDpi(metric, dpi) };
        if let Some(icon) = icon(ico, size) {
            // Posted, not sent: a hung window doesn't block the watcher.
            unsafe { PostMessageW(hwnd, WM_SETICON, kind as usize, icon as isize) };
        }
    }
}

/// The icon `ico` at `size` px, loaded once and kept.
fn icon(ico: &Path, size: i32) -> Option<HICON> {
    let key = (ico.to_path_buf(), size);
    if let Some(Some(icon)) = with_state(|s| s.icons.get(&key).copied()) {
        return Some(icon);
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
            size,
            size,
            LR_LOADFROMFILE,
        )
    } as HICON;
    if icon.is_null() {
        return None;
    }
    with_state(|s| s.icons.insert(key, icon));
    Some(icon)
}
