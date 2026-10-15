//! Running Claude Code's own command line in an instance, for [`crate::SystemEnvironment`].
//!
//! The child gets the instance's `CLAUDE_CONFIG_DIR` and loses the variables that would
//! override the instance's own login. It runs in the config directory, so no project's
//! settings get in the way, with no input: a command that would ask something fails instead
//! of waiting. Its output is kept in memory only.
//!
//! The login is the exception: it is interactive, so it runs in a terminal the user sees, and
//! mccboard reads nothing it prints.

use std::io::{self, IsTerminal, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::instance::InstanceKind;
use crate::{ClaudeInvocation, ClaudeRun, LoginExit, LoginInvocation, LoginTerminal};

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

/// Claude Code's official login, for claude.ai subscriptions.
const LOGIN_ARGS: &[&str] = &["auth", "login", "--claudeai"];

/// Opens `claude auth login` in the instance and waits for it to end. Its credentials and
/// output stay between Claude Code and the user.
pub(crate) fn login(login: &LoginInvocation) -> io::Result<LoginExit> {
    let mut command = match login.terminal {
        LoginTerminal::CurrentTerminal => {
            if !io::stdin().is_terminal() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the login needs a terminal",
                ));
            }
            let mut command = Command::new("claude");
            command.args(LOGIN_ARGS);
            command
        }
        LoginTerminal::NewWindow => new_window()?,
    };
    match login.kind {
        InstanceKind::Linked => {
            command.env("CLAUDE_CONFIG_DIR", &login.config_dir);
        }
        InstanceKind::Primary => {
            // Claude Code's default folder is `.claude` in its home: the folder's parent.
            command.env_remove("CLAUDE_CONFIG_DIR");
            if let Some(home) = login.config_dir.parent() {
                command.env(HOME_VAR, home);
            }
        }
    }
    for name in LOGIN_OVERRIDES {
        command.env_remove(name);
    }
    let status = command.current_dir(&login.config_dir).status()?;
    Ok(LoginExit {
        exit_code: status.code(),
    })
}

/// The variable holding the home directory Claude Code uses (Node's `os.homedir()`).
const HOME_VAR: &str = if cfg!(windows) { "USERPROFILE" } else { "HOME" };

/// `cmd /c start "mccboard login" /wait claude auth login --claudeai`: `start` gives the login
/// a console window of its own and `/wait` waits for it to close. `cmd` itself runs without a
/// window. The instance's folder reaches Claude Code through the environment, so no path is
/// ever quoted on a command line.
#[cfg(windows)]
fn new_window() -> io::Result<Command> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `start` would answer a missing program with a dialog box: check first.
    let found = Command::new("where.exe")
        .arg("claude")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !found.success() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "claude is not on the PATH",
        ));
    }
    let mut command = Command::new("cmd");
    command
        .raw_arg(format!(
            "/d /c start \"mccboard login\" /wait claude {}",
            LOGIN_ARGS.join(" ")
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

#[cfg(not(windows))]
fn new_window() -> io::Result<Command> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a new terminal window is opened on Windows only",
    ))
}
