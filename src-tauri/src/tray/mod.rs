//! The tray: renders the core's tray view model (icon, tooltip, menu) and keeps it current.
//!
//! One background thread is the app's only caller of [`Core::refresh_usage`]: it refreshes
//! every 60 seconds and when the user asks from the menu. The core applies the rate limits to
//! both, so neither can call the endpoint more often than they allow. Between refreshes the
//! thread redraws from the last readings when the language or `config.toml` changes.

mod icon;
mod profiles;

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mccboard_core::{Core, Language, TrayIcon, TrayProfiles, TrayView};
use tauri::image::Image;
use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "main";
/// How often the background thread asks for a refresh. The core skips accounts whose rate
/// limit or backoff doesn't allow a call yet.
const REFRESH_EVERY: Duration = Duration::from_secs(60);

const OPEN: &str = "open";
const REFRESH: &str = "refresh";
const QUIT: &str = "quit";
const LANGUAGE_PREFIX: &str = "language:";

/// What wakes the background thread.
enum Wake {
    /// Refresh usage (within the rate limits), then redraw.
    Refresh,
    /// Redraw from the last readings.
    Redraw,
}

/// Managed state: the way to wake the background thread.
struct Tray(Mutex<Sender<Wake>>);

/// Creates the tray icon and starts the thread that keeps it current.
pub(crate) fn setup(app: &tauri::App, core: &Core) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::new_owned(
            icon::render(&placeholder()),
            icon::SIZE,
            icon::SIZE,
        ))
        .tooltip("mccboard")
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_instances(tray.app_handle());
            }
        })
        .build(app)?;

    let (sender, receiver) = mpsc::channel();
    // Show the last known readings at once, before the first refresh reaches the endpoint.
    let _ = sender.send(Wake::Redraw);
    app.manage(Tray(Mutex::new(sender)));
    let handle = app.handle().clone();
    let core = core.clone();
    std::thread::Builder::new()
        .name("mccboard-tray".into())
        .spawn(move || run(&handle, &core, &receiver))?;
    Ok(())
}

/// Redraws the tray from the last readings: after the language, an account's label or color,
/// or the registered instances changed.
pub(crate) fn redraw(app: &AppHandle) {
    wake(app, Wake::Redraw);
}

/// The icon until the first redraw: no percentage, neutral gray.
fn placeholder() -> TrayIcon {
    TrayIcon {
        percent: None,
        color: "#6b7280".parse().expect("a valid color"),
        account: None,
        stale: false,
        token_expired: false,
    }
}

fn wake(app: &AppHandle, wake: Wake) {
    if let Some(tray) = app.try_state::<Tray>() {
        let sender = tray.0.lock().unwrap_or_else(|p| p.into_inner());
        let _ = sender.send(wake);
    }
}

/// The app's language: the one saved in `config.toml`, else the system's.
pub(crate) fn language(core: &Core) -> Language {
    core.language()
        .ok()
        .flatten()
        .unwrap_or_else(|| Language::from_locale(&sys_locale::get_locale().unwrap_or_default()))
}

fn run(app: &AppHandle, core: &Core, wakes: &Receiver<Wake>) {
    let mut next_refresh = Instant::now();
    loop {
        let wait = next_refresh.saturating_duration_since(Instant::now());
        let refresh = match wakes.recv_timeout(wait) {
            Ok(Wake::Refresh) | Err(RecvTimeoutError::Timeout) => true,
            Ok(Wake::Redraw) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let report = if refresh {
            next_refresh = Instant::now() + REFRESH_EVERY;
            core.refresh_usage()
        } else {
            core.usage()
        };
        let language = language(core);
        let view = report
            .and_then(|report| core.tray_view(&report, language, &jiff::tz::TimeZone::system()))
            .unwrap_or_else(|error| TrayView::unavailable(&error, language));
        // Without the profiles (a corrupt config.toml) the menu just has no "Open profile".
        let profiles = core.tray_profiles(language).ok();
        if let Err(error) = render(app, &view, profiles.as_ref()) {
            eprintln!("mccboard: cannot update the tray: {error}");
        }
    }
}

fn render(app: &AppHandle, view: &TrayView, profiles: Option<&TrayProfiles>) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    tray.set_icon(Some(Image::new_owned(
        icon::render(&view.icon),
        icon::SIZE,
        icon::SIZE,
    )))?;
    tray.set_tooltip(Some(&crate::desktop_watch::tray_tooltip(
        app,
        &view.tooltip,
    )))?;
    tray.set_menu(Some(menu(app, view, profiles)?))?;
    Ok(())
}

/// The menu: the status lines (disabled), then open Instances, open a desktop profile,
/// refresh, settings, quit.
fn menu(
    app: &AppHandle,
    view: &TrayView,
    profiles: Option<&TrayProfiles>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    crate::desktop_watch::append_to_menu(app, &menu)?;
    let line = |text: &str| MenuItem::new(app, escape(text), false, None::<&str>);
    if let Some(notice) = &view.notice {
        menu.append(&line(notice)?)?;
    }
    for (index, account) in view.accounts.iter().enumerate() {
        if index > 0 {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        menu.append(&line(&account.title)?)?;
        for text in &account.lines {
            menu.append(&line(&format!("    {text}"))?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let texts = &view.menu;
    let languages = texts
        .languages
        .iter()
        .map(|choice| {
            CheckMenuItem::with_id(
                app,
                format!("{LANGUAGE_PREFIX}{}", choice.language.code()),
                &choice.name,
                true,
                choice.selected,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let languages: Vec<&dyn IsMenuItem<tauri::Wry>> =
        languages.iter().map(|item| item as _).collect();
    let language = Submenu::with_items(app, escape(&texts.language), true, &languages)?;
    let settings = Submenu::with_items(app, escape(&texts.settings), true, &[&language])?;

    menu.append(&MenuItem::with_id(
        app,
        OPEN,
        escape(&texts.open_instances),
        true,
        None::<&str>,
    )?)?;
    if let Some(open_profile) = profiles
        .map(|profiles| profiles::submenu(app, profiles))
        .transpose()?
        .flatten()
    {
        menu.append(&open_profile)?;
    }
    menu.append_items(&[
        &MenuItem::with_id(app, REFRESH, escape(&texts.refresh), true, None::<&str>)?,
        &settings,
        &PredefinedMenuItem::separator(app)?,
        &MenuItem::with_id(app, QUIT, escape(&texts.quit), true, None::<&str>)?,
    ])?;
    Ok(menu)
}

/// Menu texts treat `&` as a mnemonic marker; labels and model names are shown as they are.
pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&&")
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        OPEN => open_instances(app),
        REFRESH => wake(app, Wake::Refresh),
        QUIT => app.exit(0),
        id if profiles::on_menu_event(app, id) => {}
        id if crate::desktop_watch::on_menu_event(app, id) => {}
        id => {
            let Some(code) = id.strip_prefix(LANGUAGE_PREFIX) else {
                return;
            };
            let Ok(language) = code.parse::<Language>() else {
                return;
            };
            let core = app.state::<Core>();
            if let Err(error) = core.set_language(language) {
                eprintln!("mccboard: cannot save the language: {error}");
            }
            // The config watcher tells the windows; the tray redraws now, checks included.
            redraw(app);
        }
    }
}

/// Shows the Instances window, creating it again if it was closed.
pub(crate) fn open_instances(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(crate::INSTANCES_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == crate::INSTANCES_WINDOW)
        .cloned();
    if let Some(config) = config {
        match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
            Ok(window) => {
                // Hidden in tauri.conf.json, so a start for a profile alone shows nothing.
                let _ = window.show();
                let _ = window.set_focus();
            }
            Err(error) => eprintln!("mccboard: cannot open the Instances window: {error}"),
        }
    }
}
