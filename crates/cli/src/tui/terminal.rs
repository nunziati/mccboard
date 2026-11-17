//! The terminal while the TUI runs (ADR 0010): raw mode and the alternate screen, entered by
//! a guard and restored when the guard is dropped (after `q` or an error), by a panic hook
//! before the panic message prints, and while an interactive program has the terminal. A
//! terminal left in raw mode makes an SSH session unusable.

use std::io::{self, Stdout};
use std::panic::{self, PanicHookInfo};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};

use anyhow::{Result, bail};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, terminal};
use ratatui::backend::{Backend, ClearType, CrosstermBackend};
use ratatui::{Frame, Terminal};

/// Switching a terminal into the TUI's modes and back.
pub trait Modes: Send + Sync {
    fn enter(&self) -> io::Result<()>;
    fn leave(&self) -> io::Result<()>;
}

/// The real terminal, through crossterm: raw mode, the alternate screen, no cursor.
pub struct Crossterm;

impl Modes for Crossterm {
    fn enter(&self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, cursor::Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(())
    }

    fn leave(&self) -> io::Result<()> {
        // Raw mode first: it changes more than the screen. Each step runs even if one fails.
        let raw = terminal::disable_raw_mode();
        let screen = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        raw.and(screen)
    }
}

/// The guards whose terminal is in the TUI's modes now, which a panic restores. The app holds
/// one at a time, tests may hold several.
static ENTERED: Mutex<Vec<(u64, Arc<dyn Modes>)>> = Mutex::new(Vec::new());

static NEXT_GUARD: AtomicU64 = AtomicU64::new(0);

fn entered() -> MutexGuard<'static, Vec<(u64, Arc<dyn Modes>)>> {
    // A panic while the lock was held must not stop the restore.
    ENTERED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Restores every terminal a guard holds in the TUI's modes.
fn restore_all() {
    let held = std::mem::take(&mut *entered());
    for (_, modes) in held {
        let _ = modes.leave();
    }
}

/// The hook a panic runs: `previous` prints the panic message, once the terminal is back.
pub type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

/// A panic hook that restores the terminal, then runs `previous`, which prints the message on
/// a screen the user can read.
pub fn restoring_hook(previous: PanicHook) -> PanicHook {
    Box::new(move |info| {
        restore_all();
        previous(info);
    })
}

/// Puts [`restoring_hook`] around the process's panic hook, once.
pub fn install_panic_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(restoring_hook(previous));
    });
}

/// Holds a terminal in the TUI's modes, and restores it when dropped.
pub struct TerminalGuard {
    id: u64,
    modes: Arc<dyn Modes>,
}

impl TerminalGuard {
    /// Puts the terminal in the TUI's modes.
    pub fn enter(modes: impl Modes + 'static) -> io::Result<TerminalGuard> {
        let guard = TerminalGuard {
            id: NEXT_GUARD.fetch_add(1, Ordering::Relaxed),
            modes: Arc::new(modes),
        };
        guard.resume()?;
        Ok(guard)
    }

    /// Whether the terminal is in the TUI's modes: not after a panic restored it, nor while an
    /// interactive program has it.
    pub fn is_entered(&self) -> bool {
        entered().iter().any(|(id, _)| *id == self.id)
    }

    /// Gives the terminal back as it was. Doing it again does nothing.
    fn suspend(&self) -> io::Result<()> {
        let held = {
            let mut entered = entered();
            let before = entered.len();
            entered.retain(|(id, _)| *id != self.id);
            entered.len() < before
        };
        if held { self.modes.leave() } else { Ok(()) }
    }

    /// Puts the terminal back in the TUI's modes.
    fn resume(&self) -> io::Result<()> {
        self.modes.enter()?;
        entered().push((self.id, Arc::clone(&self.modes)));
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

/// The terminal the TUI draws on, held in its modes by a guard.
pub struct Tui<B: Backend> {
    // Dropped first: ratatui shows the cursor again while the screen is still the TUI's.
    terminal: Terminal<B>,
    guard: TerminalGuard,
}

impl Tui<CrosstermBackend<Stdout>> {
    /// Takes over the terminal of this process. It comes back when the result is dropped, and
    /// when a panic starts.
    pub fn enter() -> Result<Tui<CrosstermBackend<Stdout>>> {
        install_panic_hook();
        let guard = TerminalGuard::enter(Crossterm)?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Tui::new(terminal, guard))
    }
}

impl<B> Tui<B>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    /// Draws on `terminal`, held by `guard`.
    pub fn new(terminal: Terminal<B>, guard: TerminalGuard) -> Tui<B> {
        Tui { terminal, guard }
    }

    /// Draws a frame. Fails once a panic has given the terminal back: no frame may cover the
    /// panic message.
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        if !self.guard.is_entered() {
            bail!("the terminal was restored after a panic");
        }
        self.terminal.draw(render)?;
        Ok(())
    }

    /// Hands the terminal to an interactive program, which `program` runs to its end (the
    /// login of Claude Code), then takes it back and draws the next frame whole. mccboard
    /// reads nothing the program prints (ADR 0002).
    pub fn release<R>(&mut self, program: impl FnOnce() -> R) -> Result<R> {
        self.guard.suspend()?;
        let result = program();
        self.guard.resume()?;
        // What `Terminal::clear` does on a full screen, without first asking the terminal
        // where its cursor is: a terminal that doesn't answer in time would end the TUI. Both
        // buffers start empty, and the next frame is drawn whole on a blank screen.
        self.terminal.backend_mut().clear_region(ClearType::All)?;
        self.terminal.swap_buffers();
        Ok(result)
    }

    /// The terminal drawn on, for tests to read what a frame shows.
    #[cfg(test)]
    pub fn terminal(&self) -> &Terminal<B> {
        &self.terminal
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use super::*;

    /// Tests that hold a guard or set the panic hook run one at a time: a panic restores every
    /// guard of the process.
    pub(crate) fn serial() -> MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Modes that write down what happened to the terminal, in order, in a shared log.
    #[derive(Clone, Default)]
    pub(crate) struct Recorded {
        pub log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Recorded {
        pub fn push(&self, what: &'static str) {
            self.log.lock().unwrap().push(what);
        }

        pub fn log(&self) -> Vec<&'static str> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Modes for Recorded {
        fn enter(&self) -> io::Result<()> {
            self.push("enter");
            Ok(())
        }

        fn leave(&self) -> io::Result<()> {
            self.push("leave");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use ratatui::backend::TestBackend;

    use super::testing::{Recorded, serial};
    use super::*;

    fn tui(modes: &Recorded) -> Tui<TestBackend> {
        let guard = TerminalGuard::enter(modes.clone()).unwrap();
        Tui::new(Terminal::new(TestBackend::new(20, 3)).unwrap(), guard)
    }

    #[test]
    fn the_terminal_comes_back_when_the_guard_is_dropped() {
        let _serial = serial();
        let modes = Recorded::default();

        let mut tui = tui(&modes);
        tui.draw(|_| {}).unwrap();
        assert_eq!(modes.log(), ["enter"]);
        drop(tui);

        assert_eq!(modes.log(), ["enter", "leave"]);
    }

    #[test]
    fn a_panic_gives_the_terminal_back_before_its_message_prints() {
        let _serial = serial();
        let modes = Recorded::default();
        let saved = panic::take_hook();
        let message = modes.clone();
        panic::set_hook(restoring_hook(Box::new(move |_| message.push("message"))));

        let tui = tui(&modes);
        let outcome = panic::catch_unwind(AssertUnwindSafe(move || {
            let _tui = tui;
            panic!("boom");
        }));

        panic::set_hook(saved);
        assert!(outcome.is_err());
        // Restored once, before the message: dropping the guard while unwinding adds nothing.
        assert_eq!(modes.log(), ["enter", "leave", "message"]);
    }

    #[test]
    fn after_a_panic_elsewhere_no_frame_covers_its_message() {
        let _serial = serial();
        let modes = Recorded::default();
        let mut tui = tui(&modes);

        // What the hook does when another thread panics.
        restore_all();

        assert!(tui.draw(|_| {}).is_err());
        drop(tui);
        assert_eq!(modes.log(), ["enter", "leave"]);
    }

    #[test]
    fn an_interactive_program_gets_the_terminal_as_it_was_and_the_tui_takes_it_back() {
        let _serial = serial();
        let modes = Recorded::default();
        let mut tui = tui(&modes);

        let program = modes.clone();
        let exit = tui
            .release(|| {
                program.push("login");
                7
            })
            .unwrap();

        assert_eq!(exit, 7);
        assert_eq!(modes.log(), ["enter", "leave", "login", "enter"]);
        tui.draw(|_| {}).unwrap();
        drop(tui);
        assert_eq!(modes.log(), ["enter", "leave", "login", "enter", "leave"]);
    }

    #[test]
    fn a_panic_in_the_interactive_program_leaves_the_terminal_as_it_was() {
        let _serial = serial();
        let modes = Recorded::default();
        let saved = panic::take_hook();
        let message = modes.clone();
        panic::set_hook(restoring_hook(Box::new(move |_| message.push("message"))));

        let mut tui = tui(&modes);
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            tui.release(|| panic!("the login failed")).unwrap();
        }));
        drop(tui);

        panic::set_hook(saved);
        assert!(outcome.is_err());
        assert_eq!(modes.log(), ["enter", "leave", "message"]);
    }
}
