//! The taskbar widget's window (Windows, #45, ADR 0009): a native window docked into the
//! primary taskbar, just left of the notification area, that draws the core's
//! [`TaskbarWidget`] and forwards clicks. It runs on its own thread inside mccboard's process.
//!
//! - **Docking**: a layered `WS_POPUP` window turned into a `WS_CHILD` of `Shell_TrayWnd` with
//!   `SetParent`. On Windows 11 `WS_EX_LAYERED` is cleared and set again right after the
//!   reparenting, or the child ends up under the taskbar's own composition. Its place comes
//!   from `GetWindowRect` of the taskbar and of `TrayNotifyWnd`: never `SHAppBarMessage`, a
//!   synchronous call into Explorer that deadlocked other tools.
//! - **Where**: only on the primary taskbar, only when it is horizontal at the bottom of its
//!   screen. When the strip would cover app buttons it is hidden until there is room again.
//!   The buttons' rectangles come from UI Automation, on a worker thread of its own: the
//!   widget's thread shares its input queue with the taskbar's once docked, so it never
//!   blocks.
//! - **Keeping up**: a window-event hook on the taskbar's thread hears the notification area
//!   move (debounced); a watchdog every [`WATCHDOG_MS`] notices an Explorer restart (the child
//!   is destroyed with its parent), a DPI or theme change, and checks the app buttons again.
//!   A hidden top-level window of the thread hears `TaskbarCreated`, `WM_SETTINGCHANGE` and
//!   `WM_DISPLAYCHANGE`, which child windows don't get.
//! - **Drawing**: into a 32-bit premultiplied bitmap, shown with `UpdateLayeredWindow`. Shapes
//!   are filled directly; text is drawn by GDI in white on black into a mask (grayscale
//!   antialiasing) and blended in its color. Every pixel keeps an alpha of at least 1, so the
//!   whole strip takes clicks.
//! - **Events**: left click, right click, and whether the widget is shown (with the taskbar's
//!   theme), for the tray: it shows the logo only while the widget is on the taskbar. The hover
//!   tooltip is a common-controls tooltip with the view's text.

use std::cell::{Cell, RefCell};
use std::io;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLACKNESS, BLENDFUNCTION, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection,
    CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_BOLD,
    FW_NORMAL, GdiFlush, GetMonitorInfoW, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints, MonitorFromWindow, OUT_DEFAULT_PRECIS,
    PatBlt, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::Controls::{
    ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASSW, TTF_IDISHWND,
    TTF_SUBCLASS, TTM_ADDTOOLW, TTM_POP, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, TTTOOLINFOW, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EVENT_OBJECT_LOCATIONCHANGE, FindWindowExW, FindWindowW, GWL_EXSTYLE, GWL_STYLE, GetClientRect,
    GetMessageW, GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, HTCLIENT,
    HWND_TOP, IsWindow, KillTimer, MA_NOACTIVATE, MSG, OBJID_WINDOW, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, RegisterClassW, RegisterWindowMessageW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageW,
    SetParent, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA,
    UpdateLayeredWindow, WINEVENT_OUTOFCONTEXT, WM_APP, WM_DISPLAYCHANGE, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_QUIT, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_TIMER, WM_USER, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::{FADED_OPACITY, SEGMENTS, TaskbarTheme, TaskbarWidget, WidgetRow};

/// How often the watchdog looks at the taskbar.
const WATCHDOG_MS: u32 = 2_000;
/// The app buttons are checked again every this many watchdog ticks, so a hidden widget comes
/// back once there is room.
const COLLISION_EVERY_TICKS: u32 = 3;
/// How long the window-event hook waits for the taskbar to settle.
const DEBOUNCE_MS: u32 = 150;
/// After Explorer restarts, the widget docks again this long after `TaskbarCreated`.
const AFTER_TASKBAR_CREATED_MS: u32 = 1_500;

const WATCHDOG_TIMER: usize = 1;
const DEBOUNCE_TIMER: usize = 2;
const TASKBAR_CREATED_TIMER: usize = 3;

/// A new view is waiting ([`TaskbarWidgetWindow::show`]).
const WM_VIEW: u32 = WM_APP + 1;
/// The worker's answer about app buttons: `wparam` 1 if they collide, `lparam` the request.
const WM_COLLISION: u32 = WM_APP + 2;
/// Lay out again later: something arrived while the thread was busy.
const WM_RELAYOUT: u32 = WM_APP + 3;

const HOST_CLASS: &str = "mccboard-taskbar-widget-host";
const WIDGET_CLASS: &str = "mccboard-taskbar-widget";

/// A click on the widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetClick {
    /// Opens the dashboard.
    Left,
    /// Opens the tray's menu.
    Right,
}

/// What the widget tells the app. Both run on the widget's thread, which shares its input with
/// the taskbar's: they must return at once (hand the work to another thread).
pub struct WidgetEvents {
    pub on_click: Box<dyn Fn(WidgetClick) + Send>,
    /// The widget appeared on the taskbar, with the taskbar's theme, or disappeared (`None`).
    /// Also called when the theme changes while it is shown.
    pub on_shown: Box<dyn Fn(Option<TaskbarTheme>) + Send>,
}

/// The widget's thread. It runs until this is dropped.
pub struct TaskbarWidgetWindow {
    thread_id: u32,
    host: isize,
    pending: Arc<Mutex<Option<TaskbarWidget>>>,
}

impl TaskbarWidgetWindow {
    /// Starts the widget's thread. Nothing shows until the first [`TaskbarWidgetWindow::show`].
    pub fn start(events: WidgetEvents) -> io::Result<TaskbarWidgetWindow> {
        let pending = Arc::new(Mutex::new(None));
        let (ready, started) = mpsc::channel();
        let thread_pending = pending.clone();
        std::thread::Builder::new()
            .name("mccboard-taskbar-widget".into())
            .spawn(move || run(events, thread_pending, ready))?;
        let (thread_id, host) = started
            .recv()
            .map_err(|_| io::Error::other("the taskbar widget's thread stopped"))?;
        if host == 0 {
            return Err(io::Error::other(
                "cannot create the taskbar widget's window",
            ));
        }
        Ok(TaskbarWidgetWindow {
            thread_id,
            host,
            pending,
        })
    }

    /// Shows `view` (or hides the widget when [`TaskbarWidget::visible`] is false).
    pub fn show(&self, view: TaskbarWidget) {
        *self.pending.lock().unwrap_or_else(|p| p.into_inner()) = Some(view);
        unsafe { PostMessageW(self.host as HWND, WM_VIEW, 0, 0) };
    }
}

impl Drop for TaskbarWidgetWindow {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
    }
}

/// The taskbar's theme: `SystemUsesLightTheme` in the user's personalization settings, dark
/// when it can't be read (Windows' default).
pub fn taskbar_theme() -> TaskbarTheme {
    let light = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"));
    match light {
        Ok(1) => TaskbarTheme::Light,
        _ => TaskbarTheme::Dark,
    }
}

/// The side of a small icon (the tray's) at the system's DPI: 16 px at 100 %, 20 at 125 %,
/// 24 at 150 %, 32 at 200 %.
pub fn small_icon_size() -> u32 {
    use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    u32::try_from(size)
        .ok()
        .filter(|&size| size > 0)
        .unwrap_or(16)
}

/// A request to the app-button worker: does `rect` (screen) cover an app button?
struct CollisionCheck {
    id: isize,
    taskbar: isize,
    rect: RECT,
}

/// Where the widget is, or would be.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Placement {
    taskbar: isize,
    taskbar_rect: (i32, i32, i32, i32),
    tray_left: i32,
    dpi: u32,
    theme: TaskbarTheme,
}

struct State {
    events: WidgetEvents,
    pending: Arc<Mutex<Option<TaskbarWidget>>>,
    view: Option<TaskbarWidget>,
    host: HWND,
    /// The docked window; null until docked, and again after Explorer destroyed it.
    hwnd: HWND,
    tooltip: HWND,
    tooltip_text: Vec<u16>,
    /// The taskbar whose thread the hook listens to.
    hooked: isize,
    hook: HWINEVENTHOOK,
    placement: Option<Placement>,
    /// The strip's rectangle on screen, shown or not.
    rect: Option<RECT>,
    /// Whether that rectangle covers app buttons: `None` until the worker answers.
    collides: Option<bool>,
    /// The latest check asked of the worker.
    check_id: isize,
    checks: Sender<CollisionCheck>,
    /// What [`WidgetEvents::on_shown`] was last told.
    reported: Option<Option<TaskbarTheme>>,
    shown: bool,
    hover: bool,
    ticks: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    /// The taskbar and its notification area, for the window-event hook's filter.
    static WATCHED: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn run(
    events: WidgetEvents,
    pending: Arc<Mutex<Option<TaskbarWidget>>>,
    ready: Sender<(u32, isize)>,
) {
    unsafe {
        // Pixels, not scaled coordinates, whatever the process says: the taskbar is per-monitor
        // aware, and the widget follows its DPI.
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let host = create_host();
        let _ = ready.send((GetCurrentThreadId(), host as isize));
        if host.is_null() {
            return;
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        InitCommonControlsEx(&controls);

        let (checks, requests) = mpsc::channel();
        let host_id = host as isize;
        let _ = std::thread::Builder::new()
            .name("mccboard-taskbar-buttons".into())
            .spawn(move || collision_worker(requests, host_id));

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(State {
                events,
                pending,
                view: None,
                host,
                hwnd: null_mut(),
                tooltip: null_mut(),
                tooltip_text: Vec::new(),
                hooked: 0,
                hook: null_mut(),
                placement: None,
                rect: None,
                collides: None,
                check_id: 0,
                checks,
                reported: None,
                shown: false,
                hover: false,
                ticks: 0,
            })
        });
        SetTimer(host, WATCHDOG_TIMER, WATCHDOG_MS, None);

        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let state = STATE.with(|cell| cell.borrow_mut().take());
        if let Some(state) = state {
            if !state.hook.is_null() {
                UnhookWinEvent(state.hook);
            }
            if !state.tooltip.is_null() {
                DestroyWindow(state.tooltip);
            }
            if !state.hwnd.is_null() {
                DestroyWindow(state.hwnd);
            }
        }
        DestroyWindow(host);
    }
}

/// The thread's hidden top-level window: timers, and the broadcasts child windows don't get.
unsafe fn create_host() -> HWND {
    unsafe {
        TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
        let instance = GetModuleHandleW(null());
        let host_class = wide(HOST_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(host_procedure),
            hInstance: instance,
            lpszClassName: host_class.as_ptr(),
            ..std::mem::zeroed()
        });
        let widget_class = wide(WIDGET_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(widget_procedure),
            hInstance: instance,
            lpszClassName: widget_class.as_ptr(),
            ..std::mem::zeroed()
        });
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            host_class.as_ptr(),
            host_class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    }
}

/// Runs `f` on the state, or, when the thread is already inside it (a message sent while it
/// waited on another window), asks for a new layout later.
fn with_state(host: HWND, f: impl FnOnce(&mut State)) {
    let done = STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => {
            if let Some(state) = state.as_mut() {
                f(state);
            }
            true
        }
        Err(_) => false,
    });
    if !done {
        unsafe { PostMessageW(host, WM_RELAYOUT, 0, 0) };
    }
}

unsafe extern "system" fn host_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED.get();
    if taskbar_created != 0 && message == taskbar_created {
        // The old taskbar is gone with the widget; dock into the new one once it settles.
        unsafe { SetTimer(hwnd, TASKBAR_CREATED_TIMER, AFTER_TASKBAR_CREATED_MS, None) };
        return 0;
    }
    match message {
        WM_VIEW => with_state(hwnd, |s| {
            let view = s.pending.lock().unwrap_or_else(|p| p.into_inner()).take();
            if let Some(view) = view {
                s.view = Some(view);
            }
            s.layout(false);
        }),
        WM_RELAYOUT | WM_SETTINGCHANGE | WM_DISPLAYCHANGE => with_state(hwnd, |s| s.layout(false)),
        WM_COLLISION => with_state(hwnd, |s| {
            if lparam == s.check_id {
                s.collides = Some(wparam != 0);
                s.layout(false);
            }
        }),
        WM_TIMER => {
            match wparam {
                DEBOUNCE_TIMER | TASKBAR_CREATED_TIMER => unsafe {
                    KillTimer(hwnd, wparam);
                },
                _ => {}
            }
            with_state(hwnd, |s| {
                s.ticks = s.ticks.wrapping_add(1);
                let recheck = wparam == WATCHDOG_TIMER && s.ticks % COLLISION_EVERY_TICKS == 0;
                s.layout(recheck);
            });
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    0
}

unsafe extern "system" fn widget_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => return HTCLIENT as LRESULT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let click = if message == WM_LBUTTONUP {
                WidgetClick::Left
            } else {
                WidgetClick::Right
            };
            STATE.with(|cell| {
                if let Ok(state) = cell.try_borrow()
                    && let Some(state) = state.as_ref()
                {
                    if !state.tooltip.is_null() {
                        unsafe { SendMessageW(state.tooltip, TTM_POP, 0, 0) };
                    }
                    (state.events.on_click)(click);
                }
            });
            return 0;
        }
        WM_MOUSEMOVE | WM_MOUSELEAVE => {
            let inside = message == WM_MOUSEMOVE;
            STATE.with(|cell| {
                if let Ok(mut state) = cell.try_borrow_mut()
                    && let Some(state) = state.as_mut()
                    && state.hover != inside
                {
                    state.hover = inside;
                    if inside {
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe { TrackMouseEvent(&mut track) };
                    }
                    state.paint();
                }
            });
            return 0;
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn on_location_change(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW || hwnd.is_null() {
        return;
    }
    let (taskbar, tray) = WATCHED.get();
    let hwnd = hwnd as isize;
    if hwnd != taskbar && hwnd != tray {
        return;
    }
    STATE.with(|cell| {
        if let Ok(state) = cell.try_borrow()
            && let Some(state) = state.as_ref()
        {
            unsafe { SetTimer(state.host, DEBOUNCE_TIMER, DEBOUNCE_MS, None) };
        }
    });
}

impl State {
    /// Docks, places, checks and draws the widget as the view and the taskbar now say; hides it
    /// when it can't or shouldn't show. `recheck` asks the worker about app buttons again.
    fn layout(&mut self, recheck: bool) {
        let visible = self.view.as_ref().is_some_and(TaskbarWidget::visible);
        let Some(placement) = (visible.then(find_placement)).flatten() else {
            self.hide();
            return;
        };
        self.hook(placement.taskbar);
        if !self.docked(placement.taskbar) && !self.dock(placement.taskbar) {
            self.hide();
            return;
        }
        self.placement = Some(placement);
        let Some(frame) = self.render() else {
            self.hide();
            return;
        };
        let (left, top, _, bottom) = placement.taskbar_rect;
        let rect = RECT {
            left: placement.tray_left - frame.width,
            top,
            right: placement.tray_left,
            bottom,
        };
        if self.rect.as_ref().map(corners) != Some(corners(&rect)) {
            self.rect = Some(rect);
            self.collides = None;
            self.check(placement.taskbar, rect);
        } else if recheck {
            self.check(placement.taskbar, rect);
        }
        // Until the worker answers about a new place, the widget stays as it was.
        let show = match self.collides {
            Some(collides) => !collides,
            None => self.shown,
        };
        if !show || rect.left < left {
            self.hide();
            return;
        }
        // The taskbar's client coordinates.
        let mut origin = POINT {
            x: rect.left,
            y: rect.top,
        };
        unsafe {
            MapWindowPoints(null_mut(), placement.taskbar as HWND, &mut origin, 1);
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                origin.x,
                origin.y,
                frame.width,
                frame.height,
                SWP_NOACTIVATE,
            );
        }
        self.present(&frame);
        self.update_tooltip();
        if !self.shown {
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            self.shown = true;
        }
        self.report(Some(placement.theme));
    }

    fn hide(&mut self) {
        if self.shown && !self.hwnd.is_null() {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
        self.shown = false;
        self.hover = false;
        self.report(None);
    }

    fn report(&mut self, shown: Option<TaskbarTheme>) {
        if self.reported != Some(shown) {
            self.reported = Some(shown);
            (self.events.on_shown)(shown);
        }
    }

    fn check(&mut self, taskbar: isize, rect: RECT) {
        self.check_id = self.check_id.wrapping_add(1);
        let _ = self.checks.send(CollisionCheck {
            id: self.check_id,
            taskbar,
            rect,
        });
    }

    /// Hooks location changes on the taskbar's thread, again after Explorer restarted.
    fn hook(&mut self, taskbar: isize) {
        let tray = unsafe {
            FindWindowExW(
                taskbar as HWND,
                null_mut(),
                wide("TrayNotifyWnd").as_ptr(),
                null(),
            )
        };
        WATCHED.set((taskbar, tray as isize));
        if self.hooked == taskbar && !self.hook.is_null() {
            return;
        }
        unsafe {
            if !self.hook.is_null() {
                UnhookWinEvent(self.hook);
            }
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(taskbar as HWND, &mut pid);
            self.hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                null_mut(),
                Some(on_location_change),
                pid,
                thread,
                WINEVENT_OUTOFCONTEXT,
            );
        }
        self.hooked = taskbar;
    }

    /// Whether the widget's window is a live child of `taskbar`.
    fn docked(&self, taskbar: isize) -> bool {
        !self.hwnd.is_null()
            && unsafe { IsWindow(self.hwnd) } != 0
            && unsafe { GetParent(self.hwnd) } as isize == taskbar
    }

    /// Makes a new window and docks it into `taskbar`.
    fn dock(&mut self, taskbar: isize) -> bool {
        unsafe {
            if !self.hwnd.is_null() && IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
            if !self.tooltip.is_null() && IsWindow(self.tooltip) != 0 {
                DestroyWindow(self.tooltip);
            }
            self.tooltip = null_mut();
            self.shown = false;
            let class = wide(WIDGET_CLASS);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            self.hwnd = hwnd;
            if hwnd.is_null() {
                return false;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let style = (style & !(WS_POPUP as isize)) | (WS_CHILD | WS_CLIPSIBLINGS) as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style);
            if SetParent(hwnd, taskbar as HWND).is_null() && GetParent(hwnd) as isize != taskbar {
                DestroyWindow(hwnd);
                self.hwnd = null_mut();
                return false;
            }
            // Windows 11 composes the reparented window under the taskbar's own visual unless
            // the layered style is set again.
            let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended & !(WS_EX_LAYERED as isize));
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended | WS_EX_LAYERED as isize);
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED | SWP_NOACTIVATE,
            );
            self.tooltip = create_tooltip(self.host, hwnd);
            self.tooltip_text.clear();
        }
        true
    }

    fn update_tooltip(&mut self) {
        let Some(view) = &self.view else {
            return;
        };
        let text = wide(&view.tooltip);
        if self.tooltip.is_null() || text == self.tooltip_text {
            return;
        }
        self.tooltip_text = text;
        let mut info = tool_info(self.hwnd);
        info.lpszText = self.tooltip_text.as_mut_ptr();
        unsafe {
            SendMessageW(
                self.tooltip,
                TTM_UPDATETIPTEXTW,
                0,
                &info as *const _ as LPARAM,
            )
        };
    }

    fn render(&self) -> Option<Frame> {
        let view = self.view.as_ref()?;
        let placement = self.placement?;
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetClientRect(placement.taskbar as HWND, &mut client) };
        let height = client.bottom - client.top;
        if height <= 0 {
            return None;
        }
        Some(Frame::draw(
            view,
            placement.theme,
            self.hover,
            placement.dpi,
            height,
        ))
    }

    /// Draws again in place: after the mouse came or left.
    fn paint(&mut self) {
        if !self.shown {
            return;
        }
        if let Some(frame) = self.render() {
            self.present(&frame);
        }
    }

    fn present(&self, frame: &Frame) {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let Some((bitmap, bits)) = dib(dc, frame.width, frame.height) else {
                DeleteDC(dc);
                return;
            };
            std::ptr::copy_nonoverlapping(frame.bgra.as_ptr(), bits, frame.bgra.len());
            let old = SelectObject(dc, bitmap as HGDIOBJ);
            let size = SIZE {
                cx: frame.width,
                cy: frame.height,
            };
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                null(),
                &size,
                dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            );
            SelectObject(dc, old);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(dc);
        }
    }
}

fn corners(rect: &RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

/// The primary taskbar, if it is horizontal at the bottom of its screen, and where the
/// notification area starts.
fn find_placement() -> Option<Placement> {
    unsafe {
        let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        if taskbar.is_null() {
            return None;
        }
        let mut rect = std::mem::zeroed::<RECT>();
        GetWindowRect(taskbar, &mut rect);
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..std::mem::zeroed()
        };
        GetMonitorInfoW(
            MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        let screen = monitor.rcMonitor;
        let horizontal = rect.right - rect.left > rect.bottom - rect.top;
        let bottom = rect.top + rect.bottom > screen.top + screen.bottom;
        if !horizontal || !bottom {
            return None;
        }
        let tray = FindWindowExW(taskbar, null_mut(), wide("TrayNotifyWnd").as_ptr(), null());
        if tray.is_null() {
            return None;
        }
        let mut tray_rect = std::mem::zeroed::<RECT>();
        GetWindowRect(tray, &mut tray_rect);
        let dpi = match GetDpiForWindow(taskbar) {
            0 => 96,
            dpi => dpi,
        };
        Some(Placement {
            taskbar: taskbar as isize,
            taskbar_rect: (rect.left, rect.top, rect.right, rect.bottom),
            tray_left: tray_rect.left,
            dpi,
            theme: taskbar_theme(),
        })
    }
}

fn tool_info(hwnd: HWND) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd,
        uId: hwnd as usize,
        ..unsafe { std::mem::zeroed() }
    }
}

/// A tooltip for the widget's window, owned by the thread's top-level window (an owner must be
/// top-level, and the taskbar belongs to Explorer).
unsafe fn create_tooltip(host: HWND, hwnd: HWND) -> HWND {
    unsafe {
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            host,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if tooltip.is_null() {
            return tooltip;
        }
        let mut empty = wide("");
        let mut info = tool_info(hwnd);
        info.lpszText = empty.as_mut_ptr();
        SendMessageW(tooltip, TTM_ADDTOOLW, 0, &info as *const _ as LPARAM);
        // Lines break at `\n` once the tooltip has a maximum width.
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, 0, 1_000);
        tooltip
    }
}

/// A top-down 32-bit DIB section of `width × height`, and its pixels.
unsafe fn dib(dc: HDC, width: i32, height: i32) -> Option<(HBITMAP, *mut u8)> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        (!bitmap.is_null() && !bits.is_null()).then_some((bitmap, bits as *mut u8))
    }
}

// --- App buttons -------------------------------------------------------------------------------

/// Answers [`CollisionCheck`]s with UI Automation, the latest one first, on its own thread.
fn collision_worker(requests: Receiver<CollisionCheck>, host: isize) {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: Option<IUIAutomation> =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
        while let Ok(mut check) = requests.recv() {
            // Only the latest place matters.
            while let Ok(newer) = requests.try_recv() {
                check = newer;
            }
            let collides = automation
                .as_ref()
                .is_some_and(|automation| covers_app_buttons(automation, &check));
            PostMessageW(host as HWND, WM_COLLISION, usize::from(collides), check.id);
        }
    }
}

/// Whether `check.rect` overlaps a button or list item of the taskbar left of the
/// notification area. Can't tell: no.
fn covers_app_buttons(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    check: &CollisionCheck,
) -> bool {
    use windows::Win32::Foundation::HWND as ComHwnd;
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
        UIA_ListItemControlTypeId,
    };

    let rect = check.rect;
    let found = unsafe {
        let Ok(taskbar) = automation.ElementFromHandle(ComHwnd(check.taskbar as _)) else {
            return false;
        };
        let condition = |kind: i32| {
            automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(kind))
        };
        let (Ok(buttons), Ok(items)) = (
            condition(UIA_ButtonControlTypeId.0),
            condition(UIA_ListItemControlTypeId.0),
        ) else {
            return false;
        };
        let Ok(either) = automation.CreateOrCondition(&buttons, &items) else {
            return false;
        };
        let Ok(found) = taskbar.FindAll(TreeScope_Descendants, &either) else {
            return false;
        };
        found
    };
    let count = unsafe { found.Length() }.unwrap_or(0);
    let taskbar_width = {
        let mut whole = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(check.taskbar as HWND, &mut whole) };
        whole.right - whole.left
    };
    (0..count).any(|index| {
        let Ok(element) = (unsafe { found.GetElement(index) }) else {
            return false;
        };
        let Ok(button) = (unsafe { element.CurrentBoundingRectangle() }) else {
            return false;
        };
        let width = button.right - button.left;
        // Empty or off-screen items, containers as wide as the taskbar, and the notification
        // area's own buttons (right of the widget) don't count.
        if width <= 0 || button.bottom <= button.top || width * 2 > taskbar_width {
            return false;
        }
        if button.left >= rect.right - 1 {
            return false;
        }
        let overlap = button.right.min(rect.right) - button.left.max(rect.left);
        let vertical = button.bottom.min(rect.bottom) - button.top.max(rect.top);
        overlap > 1 && vertical > 0
    })
}

// --- Drawing -----------------------------------------------------------------------------------

/// The strip as pixels: `width × height`, BGRA premultiplied, rows top-down.
struct Frame {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Font {
    /// The times to reset and the placeholders: Segoe UI 12.
    Normal,
    /// The row labels: Segoe UI 10.
    Small,
    /// The account labels: Segoe UI 12 bold.
    Bold,
}

/// Something to draw, in pixels.
enum Shape {
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: [u8; 4],
        opacity: f32,
    },
    Text {
        x: i32,
        y: i32,
        text: String,
        font: Font,
        color: [u8; 4],
        opacity: f32,
    },
}

/// GDI fonts at one DPI, selected into a memory DC to measure and draw text.
struct Gdi {
    dc: HDC,
    fonts: [HFONT; 3],
    original: HGDIOBJ,
}

impl Gdi {
    fn new(dpi: u32) -> Gdi {
        let px = |dips: f32| -((dips * dpi as f32 / 96.0).round() as i32);
        let face = wide("Segoe UI");
        let font = |height: i32, weight: u32| unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                // Grayscale, never ClearType: the text becomes a mask.
                u32::from(ANTIALIASED_QUALITY),
                u32::from(DEFAULT_PITCH),
                face.as_ptr(),
            )
        };
        let fonts = [
            font(px(12.0), FW_NORMAL),
            font(px(10.0), FW_NORMAL),
            font(px(12.0), FW_BOLD),
        ];
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let original = SelectObject(dc, fonts[0] as HGDIOBJ);
            Gdi {
                dc,
                fonts,
                original,
            }
        }
    }

    fn select(&self, font: Font) {
        let index = match font {
            Font::Normal => 0,
            Font::Small => 1,
            Font::Bold => 2,
        };
        unsafe { SelectObject(self.dc, self.fonts[index] as HGDIOBJ) };
    }

    /// The width and height of `text` in `font`.
    fn measure(&self, font: Font, text: &str) -> (i32, i32) {
        self.select(font);
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE { cx: 0, cy: 0 };
        unsafe { GetTextExtentPoint32W(self.dc, text.as_ptr(), text.len() as i32, &mut size) };
        (size.cx, size.cy)
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.original);
            for font in self.fonts {
                DeleteObject(font as HGDIOBJ);
            }
            DeleteDC(self.dc);
        }
    }
}

impl Frame {
    /// Lays out and draws `view` for a taskbar of `theme`, `height` pixels tall, at `dpi`.
    fn draw(
        view: &TaskbarWidget,
        theme: TaskbarTheme,
        hover: bool,
        dpi: u32,
        height: i32,
    ) -> Frame {
        let gdi = Gdi::new(dpi);
        let (width, shapes) = layout(view, theme, hover, dpi, height, &gdi);
        let bgra = rasterize(width.max(1), height, &shapes, &gdi);
        Frame {
            width: width.max(1),
            height,
            bgra,
        }
    }
}

/// The shapes of the strip, and its width. Sizes are the prototype's, in DIPs, scaled.
fn layout(
    view: &TaskbarWidget,
    theme: TaskbarTheme,
    hover: bool,
    dpi: u32,
    height: i32,
    gdi: &Gdi,
) -> (i32, Vec<Shape>) {
    let k = dpi as f32 / 96.0;
    let px = |dips: f32| (dips * k).round() as i32;
    let line = px(1.0).max(1);
    let palette = theme.palette();
    let mut shapes = Vec::new();

    // Two rows of 15, 4 apart, centred.
    let row_h = px(15.0);
    let row_gap = px(4.0);
    let top = (height - (2 * row_h + row_gap)) / 2;
    let centre = |row: i32| top + row * (row_h + row_gap) + row_h / 2;

    // The bar: a 1 px outline, 1 px inside, ten segments of 4 × 7, 1 apart.
    let segment_w = px(4.0).max(2);
    let segment_gap = line;
    let segment_h = px(7.0).max(3);
    let bar_w =
        4 * line + i32::from(SEGMENTS) * segment_w + (i32::from(SEGMENTS) - 1) * segment_gap;
    let bar_h = 4 * line + segment_h;

    let (digit_w, _) = gdi.measure(Font::Normal, "0");
    let (space_w, _) = gdi.measure(Font::Normal, " ");
    let part_gap = space_w.max(px(3.0));
    let time_width = |row: &WidgetRow| -> i32 {
        if row.reset.parts.is_empty() {
            return gdi.measure(Font::Normal, crate::TimeToReset::IDLE).0;
        }
        row.reset
            .parts
            .iter()
            .map(|part| {
                i32::from(part.slot) * digit_w + gdi.measure(Font::Normal, &part.unit.to_string()).0
            })
            .sum::<i32>()
            + part_gap * (row.reset.parts.len() as i32 - 1)
    };
    let text = |x: i32, row: i32, text: &str, font: Font, color: [u8; 4], opacity: f32| {
        let (_, h) = gdi.measure(font, text);
        Shape::Text {
            x,
            y: centre(row) - h / 2,
            text: text.to_owned(),
            font,
            color,
            opacity,
        }
    };

    let mut x = px(9.0);
    // The row labels, once at the left.
    let labels_w = view
        .row_labels
        .iter()
        .map(|label| gdi.measure(Font::Small, label).0)
        .max()
        .unwrap_or(0);
    for (row, label) in view.row_labels.iter().enumerate() {
        shapes.push(text(x, row as i32, label, Font::Small, palette.dim, 1.0));
    }
    x += labels_w;

    for (index, column) in view.columns.iter().enumerate() {
        x += px(10.0);
        if index > 0 {
            let divider_h = px(28.0);
            shapes.push(Shape::Rect {
                x,
                y: (height - divider_h) / 2,
                w: line,
                h: divider_h,
                color: palette.track,
                opacity: 1.0,
            });
            x += line + px(10.0);
        }
        let opacity = if column.state.faded() {
            FADED_OPACITY
        } else {
            1.0
        };
        let ink = column.ink(theme);
        if view.account_labels {
            let (w, h) = gdi.measure(Font::Bold, &column.label);
            shapes.push(Shape::Text {
                x,
                y: height / 2 - h / 2,
                text: column.label.clone(),
                font: Font::Bold,
                color: ink.text.rgb_with_alpha(),
                opacity,
            });
            x += w + px(6.0);
        }
        if let Some(placeholder) = column.state.placeholder() {
            // Letter-spaced, in each row, as wide as a bar at least.
            let mut widest = 0;
            for row in 0..2 {
                let mut cx = x;
                for c in placeholder.chars() {
                    let c = c.to_string();
                    shapes.push(text(cx, row, &c, Font::Normal, palette.dim, opacity));
                    cx += gdi.measure(Font::Normal, &c).0 + px(2.0);
                }
                widest = widest.max(cx - x);
            }
            x += widest.max(bar_w);
            continue;
        }
        let rows = [&column.session, &column.weekly];
        let times_w = rows.iter().map(|row| time_width(row)).max().unwrap_or(0);
        for (row, window) in rows.iter().enumerate() {
            let row = row as i32;
            let bar_y = centre(row) - bar_h / 2;
            draw_bar(
                &mut shapes,
                x,
                bar_y,
                bar_w,
                bar_h,
                line,
                window,
                ink.bar.rgb_with_alpha(),
                palette.track,
                opacity,
            );
            if let Some(pace) = window.pace {
                let tick_w = px(2.0).max(2);
                let inner = bar_w - 2 * line;
                shapes.push(Shape::Rect {
                    x: x + line + (pace as f32 * inner as f32).round() as i32 - tick_w / 2,
                    y: bar_y - px(2.0),
                    w: tick_w,
                    h: bar_h + 2 * px(2.0),
                    color: palette.text,
                    opacity: opacity * 0.85,
                });
            }
            // The time to reset, right-aligned, each number in its slot.
            let right = x + bar_w + px(7.0) + times_w;
            let mut cx = right - time_width(window);
            if window.reset.parts.is_empty() {
                shapes.push(text(
                    cx,
                    row,
                    crate::TimeToReset::IDLE,
                    Font::Normal,
                    palette.dim,
                    opacity,
                ));
                continue;
            }
            for (index, part) in window.reset.parts.iter().enumerate() {
                if index > 0 {
                    cx += part_gap;
                }
                let number = part.value.to_string();
                let slot = i32::from(part.slot) * digit_w;
                let (number_w, _) = gdi.measure(Font::Normal, &number);
                shapes.push(text(
                    cx + slot - number_w,
                    row,
                    &number,
                    Font::Normal,
                    palette.text,
                    opacity,
                ));
                cx += slot;
                let unit = part.unit.to_string();
                shapes.push(text(cx, row, &unit, Font::Normal, palette.text, opacity));
                cx += gdi.measure(Font::Normal, &unit).0;
            }
        }
        x += bar_w + px(7.0) + times_w;
    }
    let width = x + px(9.0);

    if hover {
        let hover_h = px(40.0).min(height);
        shapes.insert(
            0,
            Shape::Rect {
                x: 0,
                y: (height - hover_h) / 2,
                w: width,
                h: hover_h,
                color: palette.hover,
                opacity: 1.0,
            },
        );
    }
    (width, shapes)
}

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    shapes: &mut Vec<Shape>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    line: i32,
    window: &WidgetRow,
    color: [u8; 4],
    track: [u8; 4],
    opacity: f32,
) {
    // The outline, as four sides.
    for (sx, sy, sw, sh) in [
        (x, y, w, line),
        (x, y + h - line, w, line),
        (x, y + line, line, h - 2 * line),
        (x + w - line, y + line, line, h - 2 * line),
    ] {
        shapes.push(Shape::Rect {
            x: sx,
            y: sy,
            w: sw,
            h: sh,
            color,
            opacity,
        });
    }
    let inner_x = x + 2 * line;
    let inner_y = y + 2 * line;
    let segment_h = h - 4 * line;
    let segments = i32::from(SEGMENTS);
    let segment_w = (w - 4 * line - (segments - 1) * line) / segments;
    for index in 0..segments {
        let on = index < i32::from(window.segments);
        shapes.push(Shape::Rect {
            x: inner_x + index * (segment_w + line),
            y: inner_y,
            w: segment_w,
            h: segment_h,
            color: if on { color } else { track },
            opacity,
        });
    }
}

/// Fills the shapes into a premultiplied buffer, text through a GDI mask.
fn rasterize(width: i32, height: i32, shapes: &[Shape], gdi: &Gdi) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut pixels = vec![[0f32; 4]; w * h];
    let blend = |pixels: &mut [[f32; 4]], at: usize, color: [u8; 4], alpha: f32| {
        let alpha = alpha * f32::from(color[3]) / 255.0;
        if alpha <= 0.0 {
            return;
        }
        let pixel = &mut pixels[at];
        for channel in 0..3 {
            pixel[channel] =
                f32::from(color[channel]) / 255.0 * alpha + pixel[channel] * (1.0 - alpha);
        }
        pixel[3] = alpha + pixel[3] * (1.0 - alpha);
    };

    // The mask: white text on black, read back as coverage.
    let mask = unsafe { dib(gdi.dc, width, height) };
    let old_bitmap = mask.map(|(bitmap, _)| unsafe { SelectObject(gdi.dc, bitmap as HGDIOBJ) });
    unsafe {
        SetBkMode(gdi.dc, TRANSPARENT as i32);
        SetTextColor(gdi.dc, 0x00ff_ffff);
    }

    for shape in shapes {
        match shape {
            Shape::Rect {
                x,
                y,
                w: sw,
                h: sh,
                color,
                opacity,
            } => {
                for py in (*y).max(0)..(y + sh).min(height) {
                    for px in (*x).max(0)..(x + sw).min(width) {
                        blend(&mut pixels, py as usize * w + px as usize, *color, *opacity);
                    }
                }
            }
            Shape::Text {
                x,
                y,
                text,
                font,
                color,
                opacity,
            } => {
                let Some((_, bits)) = mask else {
                    continue;
                };
                let (tw, th) = gdi.measure(*font, text);
                let text16: Vec<u16> = text.encode_utf16().collect();
                unsafe {
                    PatBlt(gdi.dc, *x, *y, tw + 2, th, BLACKNESS);
                    TextOutW(gdi.dc, *x, *y, text16.as_ptr(), text16.len() as i32);
                    GdiFlush();
                }
                let bits = unsafe { std::slice::from_raw_parts(bits, w * h * 4) };
                for py in (*y).max(0)..(y + th).min(height) {
                    for px in (*x).max(0)..(x + tw + 2).min(width) {
                        let at = py as usize * w + px as usize;
                        let coverage = f32::from(bits[at * 4 + 1]) / 255.0;
                        blend(&mut pixels, at, *color, opacity * coverage);
                    }
                }
            }
        }
    }

    if let (Some((bitmap, _)), Some(old)) = (mask, old_bitmap) {
        unsafe {
            SelectObject(gdi.dc, old);
            DeleteObject(bitmap as HGDIOBJ);
        }
    }

    let mut bgra = Vec::with_capacity(w * h * 4);
    for [r, g, b, a] in pixels {
        // At least 1 of 255 everywhere: a fully transparent pixel lets clicks through.
        let a = a.max(1.0 / 255.0);
        let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        bgra.extend([byte(b), byte(g), byte(r), byte(a)]);
    }
    bgra
}

trait WithAlpha {
    fn rgb_with_alpha(&self) -> [u8; 4];
}

impl WithAlpha for crate::Color {
    fn rgb_with_alpha(&self) -> [u8; 4] {
        let [r, g, b] = self.rgb();
        [r, g, b, 255]
    }
}
