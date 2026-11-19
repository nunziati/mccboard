//! `mccboard setup`: the guided setup, in eight steps on one screen (stories 7 to 15 of #60):
//! Claude Code, instances, accounts, sharing, drift, launchers, preferences, and a summary
//! that opens the dashboard. Each step shows what is already done, does its work through the
//! core, and can be skipped or run again.
//!
//! - [`steps`]: each step as a plain struct, what it shows and what its keys do.
//! - [`state`]: the steps together, the one shown, and what goes out to the runner.
//! - [`view`]: how it is drawn.
//! - [`texts`]: its labels, in English and Italian.
//! - [`Runner`], here: the setup in the TUI's loop. It looks for Claude Code off the screen's
//!   thread, hands the terminal to Claude Code's installer and login and takes it back
//!   whatever their outcome, and repairs on a thread of its own, waiting for the repair to
//!   end before leaving.

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
use mccboard_core::{AddReport, AddRequest, Core, Error, LoginRun, LoginTerminal};
use ratatui::Frame;
use ratatui::backend::Backend;

use super::app::{self, Events, Flow, Screen, TerminalEvents};
use super::repairs::Repairs;
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
    let setup = Setup::new(core.clone(), super::language(core)).returning_to(back);
    let mut runner = Runner::new(setup, console)?;
    app::run(tui, events, &mut runner)?;
    Ok(runner.next)
}

/// The setup as a screen of the TUI: its state, the console for Claude Code's turns in the
/// terminal, and its threads, the one looking for Claude Code and the repairs.
pub struct Runner<'a> {
    setup: Setup,
    console: &'a mut dyn Console,
    messages: Receiver<Message>,
    sender: Sender<Message>,
    /// Looks for Claude Code: `claude --version` may take seconds.
    lookup: Option<JoinHandle<()>>,
    repairs: Repairs,
    /// What shows once the setup quits.
    pub next: Show,
}

impl<'a> Runner<'a> {
    /// The runner of `setup`. When the setup doesn't know Claude Code yet, it is looked for
    /// on a thread of its own, and the screen never waits for it.
    pub fn new(setup: Setup, console: &'a mut dyn Console) -> io::Result<Runner<'a>> {
        let (sender, messages) = mpsc::channel();
        let lookup = match setup.claude_code.claude_code {
            Some(_) => None,
            None => {
                let core = setup.core.clone();
                let sender = sender.clone();
                Some(
                    std::thread::Builder::new()
                        .name("mccboard-claude-code".into())
                        .spawn(move || {
                            let _ = sender.send(Message::ClaudeCodeFound(core.claude_code()));
                        })?,
                )
            }
        };
        Ok(Runner {
            setup,
            console,
            messages,
            sender,
            lookup,
            repairs: Repairs::default(),
            next: Show::Quit,
        })
    }

    /// The setup's state, for tests to look at.
    #[cfg(test)]
    pub fn setup(&self) -> &Setup {
        &self.setup
    }

    /// Hands the terminal to Claude Code's official installer and takes it back. The user
    /// reads what the installer printed before the setup draws again, unless nothing started.
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
            if !result.as_ref().is_err_and(nothing_ran) {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::Installed(result));
        Ok(())
    }

    /// Hands the terminal to Claude Code's login of the registered instance in `config_dir`, or
    /// with `None` to the first login of the primary instance, and takes it back. When it ran
    /// and no account logged in, or something failed after it ran, the user reads what Claude
    /// Code said first.
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
            let pause = match &result {
                Ok(report) => report.account.is_none(),
                Err(error) => !nothing_ran(error),
            };
            if pause {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::LoggedIn { config_dir, result });
        Ok(())
    }

    /// Hands the terminal to the add of a linked instance, whose login runs in it, and takes it
    /// back. When the login ran and no account logged in, or something failed after Claude
    /// Code may have run, the user reads what Claude Code said first.
    fn add<B>(&mut self, tui: &mut Tui<B>, request: AddRequest) -> Result<()>
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
            let pause = match &result {
                Ok(report) => login_ran_in_vain(report),
                Err(error) => !nothing_ran(error),
            };
            if pause {
                console.wait_for_enter(texts.back_to_setup());
            }
            result
        })?;
        self.setup.message(Message::Added(result));
        Ok(())
    }

    /// Repairs the instance in `config_dir` on a thread of its own.
    fn repair(&mut self, config_dir: PathBuf) -> Result<()> {
        let sender = self.sender.clone();
        self.repairs.start(
            self.setup.core.clone(),
            config_dir,
            move |config_dir, result| {
                let _ = sender.send(Message::Repaired { config_dir, result });
            },
        )?;
        Ok(())
    }

    /// Leaves for `to`, or, while a repair runs, waits for it to end and leaves from
    /// [`Screen::update`].
    fn leave(&mut self, to: Show) -> Flow {
        match self.repairs.running() {
            Some(dir) => {
                let dir = dir.to_path_buf();
                self.setup.leave_after_repair(to, dir);
                Flow::Continue
            }
            None => {
                self.next = to;
                Flow::Quit
            }
        }
    }
}

/// Whether `error` says that nothing ran: the core refused before starting Claude Code or its
/// installer (a folder or a label it won't take, no Claude Code, no terminal), or they couldn't
/// start. After any other error they may have printed why.
fn nothing_ran(error: &Error) -> bool {
    matches!(
        error,
        Error::NotRegistered { .. }
            | Error::InstanceFolderMissing { .. }
            | Error::ClaudeNotInstalled
            | Error::LoginNeedsTerminal { .. }
            | Error::LoginNotOpened { .. }
            | Error::NotADirectory { .. }
            | Error::RelativePath { .. }
            | Error::PrimaryInstance { .. }
            | Error::FolderNotAddable { .. }
            | Error::DesktopAppFolder { .. }
            | Error::FolderHasAnotherAccount { .. }
            | Error::DeveloperModeOff
            | Error::InstanceInUse { .. }
            | Error::InvalidLabel { .. }
            | Error::NoDefaultFolder { .. }
            | Error::FolderNotCreated { .. }
            | Error::ClaudeInstallerOnWindows
            | Error::ClaudeInstallerNeedsProgram { .. }
            | Error::ClaudeInstallerNotStarted { .. }
    )
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
            Some(Request::Quit) => return Ok(self.leave(self.setup.back)),
            Some(Request::Dashboard) => return Ok(self.leave(Show::Dashboard)),
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
        if let Some(lookup) = self.lookup.take_if(|lookup| lookup.is_finished())
            && lookup.join().is_err()
        {
            bail!("looking for Claude Code stopped unexpectedly");
        }
        self.repairs.join_ended()?;
        // The outcome of a repair joined just now.
        while let Ok(message) = self.messages.try_recv() {
            self.setup.message(message);
        }
        if let Some(to) = self.setup.leaving
            && self.repairs.running().is_none()
        {
            self.next = to;
            return Ok(Flow::Quit);
        }
        Ok(Flow::Continue)
    }

    fn interrupt(&mut self) -> Result<Flow> {
        Ok(self.leave(Show::Quit))
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests;
