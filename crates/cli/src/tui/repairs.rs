//! Repairs on threads of their own, for the screens that run them: installing a plugin runs
//! Claude Code and takes a while. A screen asked to quit while one runs waits for it, since a
//! repair stopped between moving a real item to its backup and linking it would leave the
//! instance half repaired.

use std::io;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use anyhow::{Result, bail};
use mccboard_core::{Core, Error, RepairReport};

/// The repairs a screen started, until it joins them.
#[derive(Default)]
pub struct Repairs {
    running: Vec<(PathBuf, JoinHandle<()>)>,
}

impl Repairs {
    /// Repairs every drift item of the instance in `config_dir` on a thread, and hands the
    /// outcome to `done` there.
    pub fn start(
        &mut self,
        core: Core,
        config_dir: PathBuf,
        done: impl FnOnce(PathBuf, Result<RepairReport, Error>) + Send + 'static,
    ) -> io::Result<()> {
        let dir = config_dir.clone();
        let thread = std::thread::Builder::new()
            .name("mccboard-repair".into())
            .spawn(move || {
                let result = core.repair(&dir, None);
                done(dir, result);
            })?;
        self.running.push((config_dir, thread));
        Ok(())
    }

    /// Joins the repairs whose thread ended. One that ended without handing its outcome over
    /// panicked: the panic hook gave the terminal back and printed why, and the screen stops
    /// too.
    pub fn join_ended(&mut self) -> Result<()> {
        let ended: Vec<_> = self
            .running
            .extract_if(.., |(_, thread)| thread.is_finished())
            .collect();
        for (dir, thread) in ended {
            if thread.join().is_err() {
                bail!("the repair of {} stopped unexpectedly", dir.display());
            }
        }
        Ok(())
    }

    /// The instance of a repair not joined yet, if any.
    pub fn running(&self) -> Option<&Path> {
        self.running.first().map(|(dir, _)| dir.as_path())
    }
}
