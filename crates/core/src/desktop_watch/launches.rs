//! The desktop app processes mccboard launched, kept in `~/.mccboard/state/desktop-launches.json`
//! so that the app, the CLI and a restarted mccboard all know them.
//!
//! - **Written** at every opening of a desktop profile, by whichever front end opened it. An
//!   entry is dropped at the next opening once its process is gone (neither listed nor the
//!   parent of a listed one) and it is older than [`KEEP_AT_LEAST`]; at most [`MAX`] are kept.
//! - **Read** by the watch at every poll. A file that can't be read leaves the watch with what
//!   it read last, rather than taking every running profile for one mccboard didn't start.
//!
//! The file holds pids, accounts and times: nothing from Claude's files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{AccountUuid, Environment, Paths, ProcessListing};

const FILE: &str = "desktop-launches.json";
const VERSION: u32 = 1;

/// An entry stays at least this long, whether or not its process shows yet.
const KEEP_AT_LEAST: SignedDuration = SignedDuration::from_secs(60);

/// The most entries kept: the newest.
const MAX: usize = 32;

/// A process of the desktop app that mccboard started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Launch {
    pub pid: u32,
    /// The profile's account; `None` for the default desktop profile.
    pub account: Option<AccountUuid>,
    pub at: Timestamp,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    version: u32,
    launches: Vec<Launch>,
}

fn path(paths: &Paths) -> PathBuf {
    paths.state_dir().join(FILE)
}

/// The launches remembered; an empty list when there is no file yet, `None` when it can't be
/// read or isn't valid.
pub(crate) fn load(paths: &Paths) -> Option<Vec<Launch>> {
    match fs::read(path(paths)) {
        Ok(bytes) => serde_json::from_slice::<File>(&bytes)
            .ok()
            .filter(|file| file.version == VERSION)
            .map(|file| file.launches),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(Vec::new()),
        Err(_) => None,
    }
}

/// Remembers `launch`, dropping the entries whose process is gone.
pub(crate) fn record(paths: &Paths, env: &dyn Environment, launch: Launch) -> io::Result<()> {
    let listed = env.desktop_processes();
    let now = env.now();
    let mut launches = load(paths).unwrap_or_default();
    launches.retain(|l| alive(&listed, l.pid) || now.duration_since(l.at) < KEEP_AT_LEAST);
    launches.push(launch);
    if launches.len() > MAX {
        launches.drain(..launches.len() - MAX);
    }
    save(
        &path(paths),
        &File {
            version: VERSION,
            launches,
        },
    )
}

/// Whether `pid` is listed, or is the parent of a listed process.
fn alive(listed: &[ProcessListing], pid: u32) -> bool {
    listed
        .iter()
        .any(|p| p.pid == pid || p.parent_pid == Some(pid))
}

fn save(path: &Path, file: &File) -> io::Result<()> {
    let text = serde_json::to_string_pretty(file).map_err(io::Error::other)?;
    crate::atomic_file::write(path, text.as_bytes())
}
