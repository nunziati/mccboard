//! `mccboard dashboard`: one screen with each account's usage windows and each instance's
//! state, refreshed every minute within the rate limits, as the tray and the Instances window
//! show them (story 16 to 20 of #60).
//!
//! - [`state`]: what the screen shows and what its keys do, with no terminal.
//! - [`view`]: how it is drawn.
//! - [`refresh`]: the thread that refreshes usage.
//! - [`Runner`], here: the screen in the TUI's loop, running what the state asks for.

mod refresh;
mod state;
mod view;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossterm::event::KeyEvent;
use jiff::tz::TimeZone;
use mccboard_core::Core;
use ratatui::Frame;
use ratatui::backend::Backend;

use super::app::{self, Flow, Screen, TerminalEvents};
use super::terminal::Tui;
pub use state::{Dashboard, Message, Request};

/// How often usage is refreshed, as in the tray. The core's rate limits hold for every
/// refresh, this one and the user's.
pub const REFRESH_EVERY: Duration = Duration::from_secs(60);

/// Runs the dashboard in this terminal until the user quits.
pub fn run(core: &Core) -> Result<()> {
    let language = super::language(core);
    super::require_terminal(|| {
        let texts = super::Texts::new(language);
        texts.needs_terminal("dashboard", texts.dashboard_without_terminal())
    })?;
    let dashboard = Dashboard::new(core.clone(), TimeZone::system(), language);
    let mut screen = Runner::start(dashboard, REFRESH_EVERY)?;
    let mut tui = Tui::enter()?;
    app::run(&mut tui, &mut TerminalEvents, &mut screen)
}

/// The dashboard as a screen of the TUI: its state, and the threads that do what it asks.
pub struct Runner {
    dashboard: Dashboard,
    messages: Receiver<Message>,
    sender: Sender<Message>,
    refresher: refresh::Refresher,
    repairs: Vec<(PathBuf, JoinHandle<()>)>,
    /// When the times to reset read differently.
    redraw_at: Instant,
    /// The dashboard's count of computed views when `redraw_at` was set.
    views: u64,
}

impl Runner {
    /// Starts the refresh thread: usage is refreshed at once, then every `every` and when the
    /// user asks.
    pub fn start(dashboard: Dashboard, every: Duration) -> Result<Runner> {
        let (sender, messages) = mpsc::channel();
        let refresher = refresh::spawn(dashboard.core.clone(), every, sender.clone())?;
        Ok(Runner {
            redraw_at: Instant::now() + dashboard.redraw_in(),
            views: dashboard.views(),
            dashboard,
            messages,
            sender,
            refresher,
            repairs: Vec::new(),
        })
    }

    /// Repairs the instance in `config_dir` on a thread of its own: installing a plugin runs
    /// Claude Code, which takes a while.
    fn repair(&mut self, config_dir: PathBuf) -> Result<()> {
        let core = self.dashboard.core.clone();
        let sender = self.sender.clone();
        let dir = config_dir.clone();
        let thread = std::thread::Builder::new()
            .name("mccboard-dashboard-repair".into())
            .spawn(move || {
                let result = core.repair(&dir, None);
                let _ = sender.send(Message::Repaired {
                    config_dir: dir,
                    result,
                });
            })?;
        self.repairs.push((config_dir, thread));
        Ok(())
    }

    /// Times the next redraw from views computed since the last look.
    fn follow_views(&mut self) {
        if self.dashboard.views() != self.views {
            self.views = self.dashboard.views();
            self.redraw_at = Instant::now() + self.dashboard.redraw_in();
        }
    }
}

impl<B: Backend> Screen<B> for Runner {
    fn draw(&self, frame: &mut Frame) {
        view::draw(&self.dashboard, frame);
    }

    fn key(&mut self, key: KeyEvent, _tui: &mut Tui<B>) -> Result<Flow> {
        match self.dashboard.key(key) {
            Some(Request::Quit) => return Ok(Flow::Quit),
            Some(Request::Refresh) => self.refresher.request(),
            Some(Request::Repair(config_dir)) => self.repair(config_dir)?,
            None => {}
        }
        self.follow_views();
        Ok(Flow::Continue)
    }

    fn update(&mut self) -> Result<Flow> {
        while let Ok(message) = self.messages.try_recv() {
            self.dashboard.message(message);
        }
        // A thread that ended without a word panicked: the panic hook gave the terminal back
        // and printed why, and the dashboard stops too.
        if self.refresher.stopped() {
            bail!("the usage refresh stopped unexpectedly");
        }
        let ended: Vec<_> = self
            .repairs
            .extract_if(.., |(_, thread)| thread.is_finished())
            .collect();
        for (dir, thread) in ended {
            if thread.join().is_err() {
                bail!("the repair of {} stopped unexpectedly", dir.display());
            }
        }
        if Instant::now() >= self.redraw_at {
            self.dashboard.tick();
        }
        self.follow_views();
        Ok(Flow::Continue)
    }

    fn wait(&self) -> Duration {
        self.redraw_at.saturating_duration_since(Instant::now())
    }
}

#[cfg(test)]
mod tests;
