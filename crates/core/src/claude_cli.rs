//! Running Claude Code's own command line in an instance, for [`crate::SystemEnvironment`].
//!
//! The child gets the instance's `CLAUDE_CONFIG_DIR` and loses the variables that would
//! override the instance's own login. It runs in the config directory, so no project's
//! settings get in the way, with no input: a command that would ask something fails instead
//! of waiting. Its output is kept in memory only.
//!
//! The login is the exception: it is interactive, so it runs in a terminal the user sees (the
//! CLI's own, or a new window), and mccboard reads nothing it prints.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
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
    login_with(login, &Lookup::from_env())
}

/// Where the login looks for programs: the PATH (with PATHEXT on Windows) and, for a new
/// window on Linux, `$TERMINAL`. Read from the process's environment; tests give their own.
struct Lookup {
    path: Option<OsString>,
    path_ext: Option<OsString>,
    #[cfg_attr(windows, allow(dead_code))]
    terminal: Option<OsString>,
}

impl Lookup {
    fn from_env() -> Self {
        Self {
            path: env::var_os("PATH"),
            path_ext: env::var_os("PATHEXT"),
            terminal: env::var_os("TERMINAL"),
        }
    }

    /// The program `name` as the shell would find it on the PATH: on Windows the first of
    /// `name.exe`, `name.cmd`, … in PATHEXT order (an npm install has `claude.cmd`), never a
    /// file without an extension.
    fn find(&self, name: &str) -> Option<PathBuf> {
        let dirs = env::split_paths(self.path.as_deref()?).collect::<Vec<_>>();
        let names: Vec<String> = if cfg!(windows) {
            let path_ext = self
                .path_ext
                .as_deref()
                .and_then(|ext| ext.to_str())
                .unwrap_or(".COM;.EXE;.BAT;.CMD");
            path_ext
                .split(';')
                .map(str::to_ascii_lowercase)
                .filter(|ext| matches!(ext.as_str(), ".exe" | ".com" | ".cmd" | ".bat"))
                .map(|ext| format!("{name}{ext}"))
                .collect()
        } else {
            vec![name.to_owned()]
        };
        dirs.iter()
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .find(|candidate| is_program(candidate))
    }

    /// The terminal emulator for a new window on Linux: `$TERMINAL` (a name on the PATH or a
    /// path), else `x-terminal-emulator` (Debian's choice of the user's terminal), else
    /// `xterm`. Each takes the program to run after `-e`.
    #[cfg_attr(windows, allow(dead_code))]
    fn terminal(&self) -> Option<PathBuf> {
        let chosen = self
            .terminal
            .as_deref()
            .filter(|t| !t.is_empty())
            .and_then(|terminal| {
                let path = Path::new(terminal);
                if path.is_absolute() {
                    is_program(path).then(|| path.to_path_buf())
                } else {
                    self.find(&terminal.to_string_lossy())
                }
            });
        chosen
            .or_else(|| self.find("x-terminal-emulator"))
            .or_else(|| self.find("xterm"))
    }
}

#[cfg(unix)]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_program(path: &Path) -> bool {
    path.is_file()
}

fn login_with(login: &LoginInvocation, lookup: &Lookup) -> io::Result<LoginExit> {
    // Checked first, so `NotFound` below always means Claude Code itself.
    if !login.config_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", login.config_dir.display()),
        ));
    }
    // The terminal next: without one, nothing else matters.
    let window = match login.terminal {
        LoginTerminal::CurrentTerminal if !io::stdin().is_terminal() => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the login needs a terminal",
            ));
        }
        LoginTerminal::CurrentTerminal => None,
        LoginTerminal::NewWindow => Some(window(lookup)?),
    };
    let claude = lookup
        .find("claude")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "claude is not on the PATH"))?;
    let mut command = match window {
        None => {
            let mut command = Command::new(&claude);
            // Claude Code's output goes to mccboard's standard error, which is the terminal
            // too: mccboard's standard output carries only its own report (`--json`).
            command.args(LOGIN_ARGS).stdout(io::stderr());
            command
        }
        Some(window) => new_window(&claude, window),
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

/// Claude Code itself in a console window of its own, which closes when it exits. An npm
/// install's `claude.cmd` runs through `cmd.exe /d /c` (the standard library quotes it), which
/// exits with it, so that window closes too. The instance's folder reaches Claude Code
/// through the environment, so no path is quoted on a command line.
///
/// The child keeps no standard handles of mccboard's: the app has none (it runs without a
/// console), so the new console's own are used.
#[cfg(windows)]
fn new_window(claude: &Path, _window: Console) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let mut command = Command::new(claude);
    command.args(LOGIN_ARGS).creation_flags(CREATE_NEW_CONSOLE);
    command
}

/// What opens a new window on Windows: a console of its own, always available.
#[cfg(windows)]
struct Console;

#[cfg(windows)]
fn window(_lookup: &Lookup) -> io::Result<Console> {
    Ok(Console)
}

/// What opens a new window on Linux: a terminal emulator, `Unsupported` when there is none.
#[cfg(not(windows))]
fn window(lookup: &Lookup) -> io::Result<PathBuf> {
    lookup.terminal().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "no terminal emulator found ($TERMINAL, x-terminal-emulator, xterm)",
        )
    })
}

/// Claude Code in a terminal emulator's new window: `<terminal> -e <claude> auth login …`,
/// waited for until the window closes. A terminal that hands its windows to a server and
/// returns at once (some `gnome-terminal` setups) ends the wait early: the instance then reads
/// as not logged in until "Log in" is run again or the instance is registered again.
#[cfg(not(windows))]
fn new_window(claude: &Path, terminal: PathBuf) -> Command {
    let mut command = Command::new(terminal);
    command
        .arg("-e")
        .arg(claude)
        .args(LOGIN_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    /// Writes an executable in a child `cp`, never holding it open for writing in this
    /// process (ETXTBSY: see the system environment tests).
    #[cfg(unix)]
    fn write_program(at: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let source = at.with_extension("txt");
        fs::write(&source, script).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        let status = Command::new("cp").arg(&source).arg(at).status().unwrap();
        assert!(status.success());
    }

    fn lookup(bin: &Path, terminal: Option<&str>) -> Lookup {
        Lookup {
            path: Some(bin.as_os_str().to_owned()),
            path_ext: Some(".EXE;.CMD;.JS".into()),
            terminal: terminal.map(Into::into),
        }
    }

    fn invocation(config_dir: &Path, terminal: LoginTerminal) -> LoginInvocation {
        LoginInvocation {
            config_dir: config_dir.to_path_buf(),
            kind: InstanceKind::Linked,
            terminal,
        }
    }

    #[test]
    fn without_claude_on_the_path_the_login_is_not_found() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        #[cfg(unix)]
        write_program(&bin.join("xterm"), "#!/bin/sh\nexit 0\n");

        let error = login_with(
            &invocation(home.path(), LoginTerminal::NewWindow),
            &lookup(&bin, None),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_missing_instance_folder_is_not_mistaken_for_a_missing_claude() {
        let home = tempfile::tempdir().unwrap();
        let gone = home.path().join(".claude-b");

        let error = login_with(
            &invocation(&gone, LoginTerminal::NewWindow),
            &lookup(home.path(), None),
        )
        .unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_an_npm_claude_cmd_is_found_and_a_bare_script_is_not() {
        let bin = tempfile::tempdir().unwrap();
        fs::write(bin.path().join("claude"), "#!/bin/sh\n").unwrap();
        assert_eq!(lookup(bin.path(), None).find("claude"), None);

        fs::write(bin.path().join("claude.cmd"), "@echo off\r\n").unwrap();
        assert_eq!(
            lookup(bin.path(), None).find("claude"),
            Some(bin.path().join("claude.cmd"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn on_linux_a_new_window_runs_claude_in_a_terminal_emulator_and_waits() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let out = home.path().join("out");
        // A terminal emulator that records its arguments, then runs the program after `-e`.
        write_program(
            &bin.join("my-term"),
            &format!(
                "#!/bin/sh\necho \"$@\" > '{}/term'\nshift\nexec \"$@\"\n",
                out.display()
            ),
        );
        write_program(
            &bin.join("claude"),
            &format!(
                "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$CLAUDE_CONFIG_DIR\" \"$PWD\" \"$*\" \
                 \"${{ANTHROPIC_API_KEY-unset}}\" > '{}/claude'\nexit 3\n",
                out.display()
            ),
        );
        fs::create_dir_all(&out).unwrap();

        let exit = login_with(
            &invocation(&dir, LoginTerminal::NewWindow),
            &lookup(&bin, Some("my-term")),
        )
        .unwrap();

        assert_eq!(exit.exit_code, Some(3));
        let term = fs::read_to_string(out.join("term")).unwrap();
        assert_eq!(
            term.trim(),
            format!("-e {} auth login --claudeai", bin.join("claude").display())
        );
        let claude = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(
            claude.trim(),
            format!(
                "{}|{}|auth login --claudeai|unset",
                dir.display(),
                dir.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn on_linux_without_a_terminal_emulator_a_new_window_is_unsupported() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        write_program(&bin.join("claude"), "#!/bin/sh\nexit 0\n");

        let error = login_with(
            &invocation(home.path(), LoginTerminal::NewWindow),
            &lookup(&bin, Some("no-such-terminal")),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }
}
