//! The Tauri binding: exposes core operations as commands. No logic lives here (ADR 0004).

use mccboard_core::{Candidate, Core, SystemEnvironment};
use serde::Serialize;

/// A core error as the UI sees it: a stable code it translates. No message, so nothing from
/// the core's files can reach the UI through an error.
#[derive(Debug, Serialize)]
struct CommandError {
    code: &'static str,
}

impl From<mccboard_core::Error> for CommandError {
    fn from(error: mccboard_core::Error) -> Self {
        Self { code: error.code() }
    }
}

#[tauri::command]
fn scan(core: tauri::State<'_, Core>) -> Result<Vec<Candidate>, CommandError> {
    Ok(core.scan()?)
}

pub fn run() {
    // Without a home directory the core reports its home as unreadable, which the UI shows.
    let home = std::env::home_dir().unwrap_or_default();

    tauri::Builder::default()
        .manage(Core::new(home, SystemEnvironment))
        .invoke_handler(tauri::generate_handler![scan])
        .run(tauri::generate_context!())
        .expect("error while running the mccboard app");
}
