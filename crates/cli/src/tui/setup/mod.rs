//! `mccboard setup`: the guided setup, in eight steps on one screen (stories 7 to 15 of #60):
//! Claude Code, instances, accounts, sharing, drift, launchers, preferences, and a summary
//! that opens the dashboard. Each step shows what is already done, does its work through the
//! core, and can be skipped or run again.
//!
//! - [`steps`]: each step as a plain struct, what it shows and what its keys do.
//! - [`state`]: the steps together, the one shown, and what goes out to the runner.
//! - [`view`]: how it is drawn.
//! - [`texts`]: its labels, in English and Italian.
//! - [`Runner`], here: the setup in the TUI's loop. It hands the terminal to Claude Code's
//!   installer and login and takes it back whatever their outcome, and it repairs on a thread
//!   of its own.

mod state;
mod steps;
mod texts;
mod view;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Result, bail};
use crossterm::event::KeyEvent;
use mccboard_core::{AddReport, Core, Error, LoginReport, LoginRun, LoginTerminal, OsKind};
use ratatui::Frame;
use ratatui::backend::Backend;

use super::app::{self, Events, Flow, Screen, TerminalEvents};
use super::terminal::Tui;
use super::{Show, Texts};
pub use state::{Message, Request, Setup};

/// What the setup writes in the terminal it hands to Claude Code, and how it waits for the
/// user there before taking the terminal back.
pub trait Console {
    /// Writes `text` before Claude Code starts, to say what runs and what to do.
    fn say(&mut self, text: &str);
    /// Shows `prompt`, then waits for Enter while the user reads what Claude Code printed.
    fn wait_for_enter(&mut self, prompt: &str);
}

/// The terminal itself: text on standard error, where Claude Code writes too, and Enter read
/// from standard input.
pub struct TerminalConsole;

impl Console for TerminalConsole {
    fn say(&mut self, text: &str) {
        eprintln!("\n{text}\n");
    }

    fn wait_for_enter(&mut self, prompt: &str) {
        eprint!("\n{prompt} ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
    }
}

/// Runs the setup in this terminal until the user quits. It ends on the dashboard, in the same
/// terminal.
pub fn run(core: &Core) -> Result<()> {
    let language = super::language(core);
    super::require_terminal(|| Texts::new(language).setup_needs_terminal())?;
    let mut tui = Tui::enter()?;
    super::show(
        core,
        &mut tui,
        &mut TerminalEvents,
        &mut TerminalConsole,
        Show::Setup,
    )
}

/// Shows the setup in `tui`, in the language mccboard uses now, until the user leaves it:
/// what shows next. `q` leads to `back`, the dashboard when the dashboard opened the setup.
/// The summary opens the dashboard, and Ctrl+C quits.
pub fn show<B>(
    core: &Core,
    tui: &mut Tui<B>,
    events: &mut dyn Events,
    console: &mut dyn Console,
    back: Show,
) -> Result<Show>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    let os = if cfg!(windows) {
        OsKind::Windows
    } else {
        OsKind::Linux
    };
    let setup = Setup::new(core.clone(), super::language(core), os).returning_to(back);
    let mut runner = Runner::new(setup, console);
    app::run(tui, events, &mut runner)?;
    Ok(runner.next)
}

/// The setup as a screen of the TUI: its state, the console for Claude Code's turns in the
/// terminal, and the thread of a repair.
pub struct Runner<'a> {
    setup: Setup,
    console: &'a mut dyn Console,
    messages: Receiver<Message>,
    sender: Sender<Message>,
    repair: Option<(PathBuf, JoinHandle<()>)>,
    /// What shows once the setup quits.
    pub next: Show,
}

impl<'a> Runner<'a> {
    pub fn new(setup: Setup, console: &'a mut dyn Console) -> Runner<'a> {
        let (sender, messages) = mpsc::channel();
        Runner {
            setup,
            console,
            messages,
            sender,
            repair: None,
            next: Show::Quit,
        }
    }

    /// The setup's state, for tests to look at.
    #[cfg(test)]
    pub fn setup(&self) -> &Setup {
        &self.setup
    }

    /// Hands the terminal to Claude Code's official installer and takes it back. The user
    /// reads what the installer printed before the setup draws again, unless nothing ran.
    fn install<B>(&mut self, tui: &mut Tui<B>) -> Result<()>
    where
        B: Backend,
        B::Error: Send + Sync + 'static,
    {
        let texts = self.setup.texts();
        let core = &self.setup.core;
        let console = &mut *self.console;
        let result = tui.release(|| {
            console.say(&texts.installer_starts());
            let result = core.install_claude_code();
            let ran = !matches!(
                result,
                Err(Error::ClaudeInstallerOnWindows | Error::ClaudeInstallerNeedsProgram { .. })
            );
            if ran {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::Installed(result));
        Ok(())
    }

    /// Hands the terminal to Claude Code's login of the registered instance in `config_dir`, or
    /// with `None` to the first login of the primary instance, and takes it back. When no
    /// account logged in, the user reads what Claude Code said first.
    fn login<B>(&mut self, tui: &mut Tui<B>, config_dir: Option<PathBuf>) -> Result<()>
    where
        B: Backend,
        B::Error: Send + Sync + 'static,
    {
        let texts = self.setup.texts();
        let core = &self.setup.core;
        let console = &mut *self.console;
        let first = config_dir.is_none();
        let config_dir = config_dir.unwrap_or_else(|| core.home().join(".claude"));
        let shown = self.setup.shown_path(&config_dir);
        let dir = config_dir.clone();
        let result = tui.release(|| {
            console.say(&texts.login_starts(&shown));
            let result = if first {
                core.login_primary(LoginTerminal::CurrentTerminal)
            } else {
                core.login(&dir, LoginTerminal::CurrentTerminal)
            };
            if result.as_ref().is_ok_and(not_logged_in) {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::LoggedIn { config_dir, result });
        Ok(())
    }

    /// Hands the terminal to the add of a linked instance, whose login runs in it, and takes it
    /// back. When the login ran and no account logged in, the user reads what Claude Code said
    /// first.
    fn add<B>(&mut self, tui: &mut Tui<B>, request: mccboard_core::AddRequest) -> Result<()>
    where
        B: Backend,
        B::Error: Send + Sync + 'static,
    {
        let texts = self.setup.texts();
        let core = &self.setup.core;
        let console = &mut *self.console;
        let folder = match &request.config_dir {
            Some(dir) => dir.display().to_string(),
            None => core
                .add_folder(&request.label)
                .map(|dir| self.setup.shown_path(&dir))
                .unwrap_or_default(),
        };
        let result = tui.release(|| {
            console.say(&texts.add_starts(&folder, &request.label));
            let result = core.add(&request);
            if result.as_ref().is_ok_and(login_ran_in_vain) {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::Added(result));
        Ok(())
    }

    /// Repairs the instance in `config_dir` on a thread of its own: installing a plugin runs
    /// Claude Code and takes a while.
    fn repair(&mut self, config_dir: PathBuf) -> Result<()> {
        let core = self.setup.core.clone();
        let sender = self.sender.clone();
        let dir = config_dir.clone();
        let thread = std::thread::Builder::new()
            .name("mccboard-setup-repair".into())
            .spawn(move || {
                let result = core.repair(&dir, None);
                let _ = sender.send(Message::Repaired {
                    config_dir: dir,
                    result,
                });
            })?;
        self.repair = Some((config_dir, thread));
        Ok(())
    }
}

/// Whether a login ended with no account logged in.
fn not_logged_in(report: &LoginReport) -> bool {
    report.account.is_none()
}

/// Whether an add opened the login and no account logged in.
fn login_ran_in_vain(report: &AddReport) -> bool {
    matches!(report.login.run, LoginRun::Ran { .. }) && report.login.account.is_none()
}

impl<B> Screen<B> for Runner<'_>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    fn draw(&self, frame: &mut Frame) {
        view::draw(&self.setup, frame);
    }

    fn key(&mut self, key: KeyEvent, tui: &mut Tui<B>) -> Result<Flow> {
        match self.setup.key(key) {
            None => {}
            Some(Request::Quit) => {
                self.next = self.setup.back;
                return Ok(Flow::Quit);
            }
            Some(Request::Dashboard) => {
                self.next = Show::Dashboard;
                return Ok(Flow::Quit);
            }
            Some(Request::InstallClaudeCode) => self.install(tui)?,
            Some(Request::Login(config_dir)) => self.login(tui, Some(config_dir))?,
            Some(Request::LoginPrimary) => self.login(tui, None)?,
            Some(Request::Add(request)) => self.add(tui, request)?,
            Some(Request::Repair(config_dir)) => self.repair(config_dir)?,
        }
        Ok(Flow::Continue)
    }

    fn update(&mut self) -> Result<Flow> {
        while let Ok(message) = self.messages.try_recv() {
            self.setup.message(message);
        }
        // A thread that ended without a word panicked: the panic hook gave the terminal back
        // and printed why, and the setup stops too.
        if let Some((dir, thread)) = self.repair.take_if(|(_, thread)| thread.is_finished()) {
            if thread.join().is_err() {
                bail!("the repair of {} stopped unexpectedly", dir.display());
            }
            while let Ok(message) = self.messages.try_recv() {
                self.setup.message(message);
            }
        }
        Ok(Flow::Continue)
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests;
