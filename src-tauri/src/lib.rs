//! The Tauri binding: exposes core operations as commands. No logic lives here (ADR 0004).

mod settings;
mod tray;
mod usage;

use std::path::PathBuf;
use std::time::Duration;

use mccboard_core::{
    Account, AccountUpdate, AccountUuid, Candidate, ClaudeProcess, Color, Core, Doctor,
    LauncherStatus, LauncherSync, Registration, Registry, RepairReport, SharingAnswer,
    SystemEnvironment,
};
use notify_debouncer_mini::{DebounceEventResult, new_debouncer, notify::RecursiveMode};
use serde::Serialize;
use tauri::{Emitter, Manager};

/// Emitted to the windows whenever `config.toml` changes on disk, whoever changed it (the CLI,
/// a text editor, this app).
const CONFIG_CHANGED: &str = "config-changed";

/// The label of the Instances window in `tauri.conf.json`.
const INSTANCES_WINDOW: &str = "instances";

/// A core error as the UI sees it: a stable code it translates, plus the path it is about and,
/// for a corrupt `config.toml`, where it is wrong. Nothing from Claude Code's files can reach
/// the UI through an error.
#[derive(Debug, Serialize)]
struct CommandError {
    code: &'static str,
    path: Option<PathBuf>,
    detail: Option<String>,
    /// For `instance_in_use`: the Claude Code processes using the instance.
    processes: Option<Vec<ClaudeProcess>>,
}

impl CommandError {
    /// A failure outside the core (a worker thread that died): the UI's generic message.
    fn unknown() -> Self {
        Self {
            code: "unknown",
            path: None,
            detail: None,
            processes: None,
        }
    }
}

impl From<mccboard_core::Error> for CommandError {
    fn from(error: mccboard_core::Error) -> Self {
        Self {
            code: error.code(),
            path: error.path().map(PathBuf::from),
            detail: error.detail().map(str::to_owned),
            processes: error.processes().map(<[ClaudeProcess]>::to_vec),
        }
    }
}

#[tauri::command]
fn scan(core: tauri::State<'_, Core>) -> Result<Vec<Candidate>, CommandError> {
    Ok(core.scan()?)
}

#[tauri::command]
fn registry(core: tauri::State<'_, Core>) -> Result<Registry, CommandError> {
    Ok(core.registry()?)
}

#[tauri::command]
fn register(core: tauri::State<'_, Core>, path: PathBuf) -> Result<Registration, CommandError> {
    Ok(core.register(&path)?)
}

#[tauri::command]
fn unregister(core: tauri::State<'_, Core>, path: PathBuf) -> Result<(), CommandError> {
    Ok(core.unregister(&path)?)
}

/// `launcher`: `None` leaves it, `Some("")` drops the override, anything else sets it.
#[tauri::command]
fn update_account(
    core: tauri::State<'_, Core>,
    uuid: String,
    label: Option<String>,
    color: Option<String>,
    launcher: Option<String>,
) -> Result<Account, CommandError> {
    let update = AccountUpdate {
        label,
        color: color.map(|c| c.parse::<Color>()).transpose()?,
        launcher: launcher.map(|name| Some(name).filter(|name| !name.is_empty())),
    };
    Ok(core.update_account(&AccountUuid::new(uuid), update)?)
}

#[tauri::command]
fn doctor(core: tauri::State<'_, Core>) -> Result<Doctor, CommandError> {
    Ok(core.doctor()?)
}

#[tauri::command]
fn answer_sharing(
    core: tauri::State<'_, Core>,
    item: String,
    answer: SharingAnswer,
) -> Result<(), CommandError> {
    Ok(core.answer_sharing(&item, answer)?)
}

#[tauri::command]
fn launchers(core: tauri::State<'_, Core>) -> Result<LauncherStatus, CommandError> {
    Ok(core.launchers()?)
}

#[tauri::command]
fn sync_launchers(core: tauri::State<'_, Core>) -> Result<LauncherSync, CommandError> {
    Ok(core.sync_launchers()?)
}

/// Repairs an instance's drift: every item, or only `item` (a drift item id).
#[tauri::command]
fn repair(
    core: tauri::State<'_, Core>,
    path: PathBuf,
    item: Option<String>,
) -> Result<RepairReport, CommandError> {
    Ok(core.repair(&path, item.as_deref())?)
}

/// Watches `~/.mccboard/` (the folder, since editors and the core save by renaming) and tells
/// the windows when `config.toml` changes.
fn watch_config(app: &tauri::App, core: &Core) -> Result<(), Box<dyn std::error::Error>> {
    let paths = core.paths();
    std::fs::create_dir_all(paths.root())?;
    let handle = app.handle().clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |result: DebounceEventResult| {
            let changed = result.is_ok_and(|events| {
                events
                    .iter()
                    .any(|e| e.path.file_name() == Some("config.toml".as_ref()))
            });
            if changed {
                let _ = handle.emit(CONFIG_CHANGED, ());
                tray::redraw(&handle);
            }
        },
    )?;
    debouncer
        .watcher()
        .watch(paths.root(), RecursiveMode::NonRecursive)?;
    // Managed state lives as long as the app, and the watcher with it.
    app.manage(debouncer);
    Ok(())
}

pub fn run() {
    // Without a home directory the core reports its home as unreadable, which the UI shows.
    let home = std::env::home_dir().unwrap_or_default();
    let core = Core::new(home, SystemEnvironment);

    tauri::Builder::default()
        .manage(core.clone())
        .invoke_handler(tauri::generate_handler![
            scan,
            registry,
            register,
            unregister,
            update_account,
            doctor,
            answer_sharing,
            usage::usage,
            usage::refresh_usage,
            launchers,
            sync_launchers,
            repair,
            settings::language,
            settings::set_language
        ])
        .setup(move |app| {
            tray::setup(app, &core)?;
            // Without the watcher the window still works; it just won't see outside edits
            // until it reloads.
            if let Err(error) = watch_config(app, &core) {
                eprintln!("mccboard: cannot watch config.toml: {error}");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the mccboard app")
        .run(|_app, event| {
            // Closing the last window leaves mccboard in the tray; Quit in its menu exits.
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
