//! Running Claude Code's own command line in an instance, for [`crate::SystemEnvironment`].
//!
//! The child gets the instance's `CLAUDE_CONFIG_DIR` and loses the variables that would
//! override the instance's own login. It runs in the config directory, so no project's
//! settings get in the way, with no input: a command that would ask something fails instead
//! of waiting. Its output is kept in memory only.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{ClaudeInvocation, ClaudeRun};

/// Variables that make Claude Code use another login than the instance's.
const LOGIN_OVERRIDES: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];

/// Long enough to clone a marketplace over a slow network.
const TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) fn run(invocation: &ClaudeInvocation) -> io::Result<ClaudeRun> {
    let mut command = Command::new("claude");
    command
        .args(&invocation.args)
        .env("CLAUDE_CONFIG_DIR", &invocation.config_dir)
        .current_dir(&invocation.config_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in LOGIN_OVERRIDES {
        command.env_remove(name);
    }
    let mut child = command.spawn()?;

    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Claude Code didn't finish within 10 minutes",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };
    let collect = |reader: Option<thread::JoinHandle<String>>| {
        reader
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default()
    };
    Ok(ClaudeRun {
        exit_code: status.code(),
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

/// Reads a pipe to the end on its own thread, so a full pipe never blocks the child.
fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}
