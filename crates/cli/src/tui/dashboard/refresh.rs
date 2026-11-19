//! The dashboard's usage refresh, on a thread of its own as in the tray: at start, every
//! [`super::REFRESH_EVERY`] and when the user asks. Every refresh goes through
//! [`Core::refresh_usage`], whose rate limits hold whatever asks for it: at most one call to the
//! usage endpoint per account per minute, the tray's budget (ADR 0002, ADR 0003).

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mccboard_core::Core;

use super::state::Message;

/// The refresh thread, and the way to ask it for a refresh.
pub struct Refresher {
    requests: Sender<()>,
    thread: JoinHandle<()>,
}

impl Refresher {
    /// Asks for a refresh now. Asking again while one runs adds nothing: the next refresh
    /// serves every request made before it starts.
    pub fn request(&self) {
        let _ = self.requests.send(());
    }

    /// Whether the thread ended, which only a panic makes it do while the dashboard runs.
    pub fn stopped(&self) -> bool {
        self.thread.is_finished()
    }
}

/// Starts the refresh thread: it refreshes at once, then every `every` and on request, and
/// sends each outcome to `messages`. It ends when the dashboard drops its [`Refresher`] or its
/// receiver.
pub fn spawn(core: Core, every: Duration, messages: Sender<Message>) -> io::Result<Refresher> {
    let (requests, wakes) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("mccboard-dashboard-refresh".into())
        .spawn(move || run(&core, every, &wakes, &messages))?;
    Ok(Refresher { requests, thread })
}

fn run(core: &Core, every: Duration, wakes: &Receiver<()>, messages: &Sender<Message>) {
    let mut next = Instant::now();
    loop {
        match wakes.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while wakes.try_recv().is_ok() {}
        next = Instant::now() + every;
        if messages.send(Message::RefreshStarted).is_err() {
            return;
        }
        if messages.send(Message::Usage(core.refresh_usage())).is_err() {
            return;
        }
    }
}
