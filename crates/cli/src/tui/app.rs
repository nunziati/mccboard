//! The loop every screen of the TUI runs in: draw, wait for a key or for the screen's next
//! update, hand the key to the screen, let it update, again until it quits.
//!
//! Keys are read on the loop's own thread, never in the background: an interactive program
//! the screen hands the terminal to ([`Tui::release`]) must get every key the user types.

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::backend::Backend;

use super::terminal::Tui;

/// Whether the loop goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Where the loop's events come from: the terminal, or a script in tests.
pub trait Events {
    /// The next event, waiting at most `timeout`, or `None` when none came.
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// The terminal's events, through crossterm.
pub struct TerminalEvents;

impl Events for TerminalEvents {
    fn next(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }
}

/// A screen of the TUI: its state, how it draws, what its keys do.
pub trait Screen<B: Backend> {
    fn draw(&self, frame: &mut Frame);

    /// A key was pressed. Through `tui` the screen may hand the terminal to an interactive
    /// program.
    fn key(&mut self, key: KeyEvent, tui: &mut Tui<B>) -> Result<Flow>;

    /// Runs after every wait, key or not: takes what the screen's workers sent and does what
    /// is due.
    fn update(&mut self) -> Result<Flow>;

    /// The user pressed Ctrl+C: the loop ends, unless the screen has work under way that
    /// must end first (a repair). It then quits from [`Screen::update`] once the work ends.
    fn interrupt(&mut self) -> Result<Flow> {
        Ok(Flow::Quit)
    }

    /// How long the loop may wait for a key before [`Screen::update`] is due.
    fn wait(&self) -> Duration;
}

/// The longest wait for a key: what workers send must show at once.
const LONGEST_WAIT: Duration = Duration::from_millis(200);

/// Runs `screen` until it quits, or until the user presses Ctrl+C (in raw mode a key like any
/// other) and the screen lets the loop end. An error ends the loop and returns.
pub fn run<B>(tui: &mut Tui<B>, events: &mut dyn Events, screen: &mut dyn Screen<B>) -> Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    loop {
        tui.draw(|frame| screen.draw(frame))?;
        let wait = screen.wait().min(LONGEST_WAIT);
        // Windows reports releases too: a key acts once, when pressed (or held).
        if let Some(Event::Key(key)) = events.next(wait)?
            && key.kind != KeyEventKind::Release
        {
            let flow = if is_interrupt(&key) {
                screen.interrupt()?
            } else {
                screen.key(key, tui)?
            };
            if flow == Flow::Quit {
                return Ok(());
            }
        }
        if screen.update()? == Flow::Quit {
            return Ok(());
        }
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

/// Whether `code` answers yes to a question: y, s (sì) or Enter.
pub fn is_yes(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('y' | 'Y' | 's' | 'S') | KeyCode::Enter)
}

/// Whether `code` answers no to a question: n or Esc.
pub fn is_no(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('n' | 'N') | KeyCode::Esc)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::VecDeque;
    use std::time::Instant;

    use super::*;

    /// One step of a scripted user.
    pub(crate) enum Step {
        /// Presses a key.
        Key(KeyCode),
        /// Presses a key with modifiers.
        Press(KeyEvent),
        /// Waits for `until` to hold, giving up after a few seconds.
        Until(&'static str, Box<dyn Fn() -> bool>),
        /// Does something outside the TUI (moves the fake clock).
        Do(Box<dyn FnOnce()>),
        /// The terminal fails.
        Fail,
    }

    /// Events from a script of steps. Once the script is over, it quits.
    pub(crate) struct Script {
        steps: VecDeque<Step>,
        waiting_since: Option<Instant>,
    }

    impl Script {
        pub fn new(steps: Vec<Step>) -> Script {
            Script {
                steps: steps.into(),
                waiting_since: None,
            }
        }
    }

    pub(crate) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    impl Events for Script {
        fn next(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
            loop {
                match self.steps.pop_front() {
                    None => return Ok(Some(Event::Key(key(KeyCode::Char('q'))))),
                    Some(Step::Key(code)) => return Ok(Some(Event::Key(key(code)))),
                    Some(Step::Press(key)) => return Ok(Some(Event::Key(key))),
                    Some(Step::Do(action)) => action(),
                    Some(Step::Fail) => return Err(io::Error::other("the terminal went away")),
                    Some(Step::Until(what, until)) => {
                        if until() {
                            self.waiting_since = None;
                            continue;
                        }
                        let since = *self.waiting_since.get_or_insert_with(Instant::now);
                        assert!(
                            since.elapsed() < Duration::from_secs(10),
                            "still waiting for {what}"
                        );
                        self.steps.push_front(Step::Until(what, until));
                        std::thread::sleep(timeout.min(Duration::from_millis(10)));
                        return Ok(None);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::testing::{Script, Step};
    use super::*;
    use crate::tui::terminal::TerminalGuard;
    use crate::tui::terminal::testing::{Recorded, serial};

    /// Counts keys and updates, and quits on `q`.
    #[derive(Default)]
    struct Counter {
        keys: Cell<usize>,
        updates: usize,
        fail_on_update: bool,
    }

    impl Screen<TestBackend> for Counter {
        fn draw(&self, frame: &mut Frame) {
            frame.render_widget(format!("keys {}", self.keys.get()), frame.area());
        }

        fn key(&mut self, key: KeyEvent, _tui: &mut Tui<TestBackend>) -> Result<Flow> {
            self.keys.set(self.keys.get() + 1);
            Ok(match key.code {
                KeyCode::Char('q') => Flow::Quit,
                _ => Flow::Continue,
            })
        }

        fn update(&mut self) -> Result<Flow> {
            self.updates += 1;
            if self.fail_on_update {
                anyhow::bail!("the screen failed");
            }
            Ok(Flow::Continue)
        }

        fn wait(&self) -> Duration {
            Duration::from_millis(1)
        }
    }

    fn tui(modes: &Recorded) -> Tui<TestBackend> {
        let guard = TerminalGuard::enter(modes.clone()).unwrap();
        Tui::new(Terminal::new(TestBackend::new(20, 2)).unwrap(), guard)
    }

    #[test]
    fn the_loop_hands_keys_to_the_screen_until_it_quits_and_the_terminal_comes_back() {
        let _serial = serial();
        let modes = Recorded::default();
        let mut screen = Counter::default();
        let mut events = Script::new(vec![
            Step::Key(KeyCode::Down),
            Step::Key(KeyCode::Char('x')),
            Step::Key(KeyCode::Char('q')),
        ]);

        {
            let mut tui = tui(&modes);
            run(&mut tui, &mut events, &mut screen).unwrap();
            let shown = tui.terminal().backend().buffer().content[..6]
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert_eq!(shown, "keys 2");
        }

        assert_eq!(screen.keys.get(), 3);
        assert_eq!(modes.log(), ["enter", "leave"]);
    }

    #[test]
    fn ctrl_c_quits_whatever_the_screen() {
        let _serial = serial();
        let modes = Recorded::default();
        let mut screen = Counter::default();
        let interrupt = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut events = Script::new(vec![Step::Press(interrupt), Step::Key(KeyCode::Down)]);

        let mut tui = tui(&modes);
        run(&mut tui, &mut events, &mut screen).unwrap();

        assert_eq!(screen.keys.get(), 0);
    }

    #[test]
    fn an_error_ends_the_loop_and_the_terminal_still_comes_back() {
        let _serial = serial();
        for fail_on_update in [false, true] {
            let modes = Recorded::default();
            let mut screen = Counter {
                fail_on_update,
                ..Counter::default()
            };
            let mut events = Script::new(vec![Step::Fail]);

            let result = {
                let mut tui = tui(&modes);
                run(&mut tui, &mut events, &mut screen)
            };

            assert!(result.is_err());
            assert_eq!(modes.log(), ["enter", "leave"]);
        }
    }
}
