//! Running Claude Code's own command line in an instance, for [`crate::SystemEnvironment`].
//!
//! The child gets the instance's `CLAUDE_CONFIG_DIR` and loses the variables that would
//! override the instance's own login. It runs in the config directory, so no project's
//! settings get in the way, with no input: a command that would ask something fails instead
//! of waiting. Its output is kept in memory only, and on Windows it opens no console window
//! ([`crate::background`]).
//!
//! The login is the exception: it is interactive, so it runs in a terminal the user sees (the
//! CLI's own, or a new window), and mccboard reads nothing it prints.
//!
//! The token renewal (`claude mcp get` with a missing name) runs hidden like a background run,
//! but in a folder of mccboard's own and with its output discarded.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::instance::InstanceKind;
use crate::{
    ClaudeInvocation, ClaudeRun, LoginExit, LoginInvocation, LoginTerminal, RenewalExit,
    RenewalInvocation,
};

/// Variables that make Claude Code use another login than the instance's.
const LOGIN_OVERRIDES: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];

/// Long enough to clone a marketplace over a slow network.
const TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) fn run(invocation: &ClaudeInvocation) -> io::Result<ClaudeRun> {
    run_with(invocation, &Lookup::from_env())
}

/// [`run`] with `claude` looked up like the login's, so both find the same program: on
/// Windows an npm install's `claude.cmd` too, which std runs through `cmd.exe /d /c` in the
/// same hidden console.
fn run_with(invocation: &ClaudeInvocation, lookup: &Lookup) -> io::Result<ClaudeRun> {
    let claude = lookup.find("claude").ok_or_else(not_on_path)?;
    let mut command = crate::background::command(claude);
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

fn not_on_path() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "claude is not on the PATH")
}

/// Where Claude Code and, for a new login window on Linux, the terminal are looked for: the
/// PATH (with PATHEXT on Windows) and `$TERMINAL`. Read from the process's environment; tests
/// give their own.
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
    /// file without an extension, the same lookup as the launchers'
    /// ([`mccboard_shim::find_program`]).
    fn find(&self, name: &str) -> Option<PathBuf> {
        let path = self.path.as_deref()?;
        if cfg!(windows) {
            mccboard_shim::find_program(name, path, self.path_ext.as_deref(), None)
        } else {
            env::split_paths(path)
                .map(|dir| dir.join(name))
                .find(|candidate| is_program(candidate))
        }
    }

    /// The terminal emulator for a new window on Linux: `$TERMINAL` (a name on the PATH or a
    /// path), else `x-terminal-emulator` (Debian's choice of the user's terminal), else the
    /// first of [`TERMINALS`] on the PATH. A desktop started from its own menu often has
    /// neither of the first two.
    #[cfg_attr(windows, allow(dead_code))]
    fn terminal(&self) -> Option<Terminal> {
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
        let program = chosen
            .or_else(|| self.find("x-terminal-emulator"))
            .or_else(|| TERMINALS.iter().find_map(|(name, _)| self.find(name)))?;
        Some(Terminal::new(program))
    }
}

/// The terminal emulators mccboard knows, in the order it tries them, each with what it takes
/// before the program to run. Each runs the program in a process of its own that ends when
/// its window closes, so the login can be waited for.
#[cfg_attr(windows, allow(dead_code))]
const TERMINALS: &[(&str, &[&str])] = &[
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--always-new-process", "--"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--wait", "--"]),
    ("xfce4-terminal", &["--disable-server", "-x"]),
    ("xterm", &["-e"]),
];

/// What any other terminal takes before the program: `-e`, which Debian requires of every
/// `x-terminal-emulator`.
#[cfg_attr(windows, allow(dead_code))]
const DASH_E: &[&str] = &["-e"];

/// A terminal emulator for a new window on Linux.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Debug)]
struct Terminal {
    program: PathBuf,
    /// The arguments before the program to run: the known terminal's own, else `-e`.
    before_program: &'static [&'static str],
}

#[cfg_attr(windows, allow(dead_code))]
impl Terminal {
    /// `program` with the arguments its file name calls for.
    fn new(program: PathBuf) -> Self {
        let name = program.file_name().and_then(|name| name.to_str());
        let before_program = TERMINALS
            .iter()
            .find(|(known, _)| Some(*known) == name)
            .map_or(DASH_E, |(_, args)| *args);
        Self {
            program,
            before_program,
        }
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

/// Claude Code's details of an MCP server, asked for a name no server has (#51). Before looking
/// it up, Claude Code loads the account's claude.ai connectors, and for that it waits until it
/// has renewed an expired token. Then it finds no such server and exits: no MCP server is
/// started, no model called, no usage counted. `auth status` doesn't wait: it exits before its startup
/// refresh is done, and usually leaves the token expired.
const RENEW_ARGS: &[&str] = &["mcp", "get", "mccboard-renewal-no-such-server"];

/// How `mcp get` ends when it finds no such server: the expected end, reported as 0.
const NO_SUCH_SERVER_EXIT: i32 = 1;

/// About a second when it works; a minute covers a slow network.
const RENEW_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `claude mcp get` with a missing name in the instance, in the background, so Claude Code
/// renews its own expired token. Its output goes nowhere: only the exit code is kept.
pub(crate) fn renew(renewal: &RenewalInvocation) -> io::Result<RenewalExit> {
    renew_with(renewal, &Lookup::from_env(), RENEW_TIMEOUT)
}

fn renew_with(
    renewal: &RenewalInvocation,
    lookup: &Lookup,
    timeout: Duration,
) -> io::Result<RenewalExit> {
    let claude = lookup.find("claude").ok_or_else(not_on_path)?;
    let mut command = crate::background::command(claude);
    command
        .args(RENEW_ARGS)
        .current_dir(&renewal.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    in_instance(&mut command, &renewal.config_dir, renewal.kind);
    let mut child = command.spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Claude Code didn't finish its renewal in time",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(RenewalExit {
        exit_code: status.code().map(|code| match code {
            NO_SUCH_SERVER_EXIT => 0,
            code => code,
        }),
    })
}

/// Points `command` at the instance in `config_dir`, as the login does: `CLAUDE_CONFIG_DIR`
/// for a linked instance; for the primary instance none, with the home set to its folder's
/// parent. The variables that would override the instance's own login are removed.
fn in_instance(command: &mut Command, config_dir: &Path, kind: InstanceKind) {
    match kind {
        InstanceKind::Linked => {
            command.env("CLAUDE_CONFIG_DIR", config_dir);
        }
        InstanceKind::Primary => {
            // Claude Code's default folder is `.claude` in its home: the folder's parent.
            command.env_remove("CLAUDE_CONFIG_DIR");
            if let Some(home) = config_dir.parent() {
                command.env(HOME_VAR, home);
            }
        }
    }
    for name in LOGIN_OVERRIDES {
        command.env_remove(name);
    }
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
    let claude = lookup.find("claude").ok_or_else(not_on_path)?;
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
    in_instance(&mut command, &login.config_dir, login.kind);
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
    use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

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
fn window(lookup: &Lookup) -> io::Result<Terminal> {
    lookup.terminal().ok_or_else(|| {
        let known: Vec<&str> = TERMINALS.iter().map(|(name, _)| *name).collect();
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "no terminal emulator found ($TERMINAL, x-terminal-emulator, {})",
                known.join(", ")
            ),
        )
    })
}

/// Claude Code in a terminal emulator's new window, `<terminal> -e <claude> auth login …` or
/// what that terminal takes instead of `-e`, waited for until the window closes. A terminal
/// that hands its windows to a server and returns at once ends the wait early: the instance
/// then reads as not logged in until "Log in" is run again or the instance is registered
/// again. The known ones are started so that they don't.
#[cfg(not(windows))]
fn new_window(claude: &Path, terminal: Terminal) -> Command {
    let mut command = Command::new(terminal.program);
    command
        .args(terminal.before_program)
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

    fn claude_invocation(config_dir: &Path) -> ClaudeInvocation {
        ClaudeInvocation {
            config_dir: config_dir.to_path_buf(),
            args: vec!["plugin".into(), "list".into()],
        }
    }

    #[test]
    fn without_claude_on_the_path_a_background_run_is_not_found() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();

        let error = run_with(&claude_invocation(home.path()), &lookup(&bin, None)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_run_uses_the_claude_found_on_the_path_in_the_instance() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&dir).unwrap();
        write_program(
            &bin.join("claude"),
            "#!/bin/sh\necho \"$CLAUDE_CONFIG_DIR|$PWD|$*\"\nexit 3\n",
        );

        let run = run_with(&claude_invocation(&dir), &lookup(&bin, None)).unwrap();

        assert_eq!(run.exit_code, Some(3));
        assert_eq!(
            run.stdout.trim(),
            format!("{}|{}|plugin list", dir.display(), dir.display())
        );
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_background_run_starts_an_npm_claude_cmd() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&dir).unwrap();
        // npm's stand-in: a batch file only, which std runs through cmd.exe.
        fs::write(
            bin.join("claude.cmd"),
            "@echo off\r\necho %CLAUDE_CONFIG_DIR%^|%*\r\nexit /b 3\r\n",
        )
        .unwrap();

        let run = run_with(&claude_invocation(&dir), &lookup(&bin, None)).unwrap();

        assert_eq!(run.exit_code, Some(3));
        assert_eq!(run.stdout.trim(), format!("{}|plugin list", dir.display()));
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

    fn renewal(config_dir: &Path, kind: InstanceKind, working_dir: &Path) -> RenewalInvocation {
        RenewalInvocation {
            config_dir: config_dir.to_path_buf(),
            kind,
            working_dir: working_dir.to_path_buf(),
        }
    }

    /// A home with a `bin` folder, a linked instance `.claude-b`, the neutral folder and an
    /// `out` folder where fake programs write what they saw.
    fn renewal_home() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let [bin, dir, neutral, out] = ["bin", ".claude-b", "neutral", "out"].map(|name| {
            let path = home.path().join(name);
            fs::create_dir_all(&path).unwrap();
            path
        });
        (home, bin, dir, neutral, out)
    }

    #[test]
    fn without_claude_on_the_path_a_renewal_is_not_found() {
        let (_home, bin, dir, neutral, _out) = renewal_home();

        let error = renew_with(
            &renewal(&dir, InstanceKind::Linked, &neutral),
            &lookup(&bin, None),
            RENEW_TIMEOUT,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn a_renewal_runs_mcp_get_in_the_instance_from_the_neutral_folder() {
        let (_home, bin, dir, neutral, out) = renewal_home();
        write_program(
            &bin.join("claude"),
            &format!(
                "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"${{CLAUDE_CONFIG_DIR-unset}}\" \"$PWD\" \
                 \"$*\" \"${{ANTHROPIC_API_KEY-unset}}\" > '{}/claude'\n\
                 echo 'No MCP server named \"x\".' >&2\nexit 1\n",
                out.display()
            ),
        );

        let exit = renew_with(
            &renewal(&dir, InstanceKind::Linked, &neutral),
            &lookup(&bin, None),
            RENEW_TIMEOUT,
        )
        .unwrap();

        // Finding no such server is how it ends: reported as 0.
        assert_eq!(exit.exit_code, Some(0));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(
            seen.trim(),
            format!(
                "{}|{}|{}|unset",
                dir.display(),
                neutral.display(),
                RENEW_ARGS.join(" ")
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_renewal_of_the_primary_instance_sets_the_home_not_the_config_dir() {
        let (home, bin, _dir, neutral, out) = renewal_home();
        let primary = home.path().join(".claude");
        fs::create_dir_all(&primary).unwrap();
        write_program(
            &bin.join("claude"),
            &format!(
                "#!/bin/sh\nprintf '%s|%s\\n' \"${{CLAUDE_CONFIG_DIR-unset}}\" \"$HOME\" \
                 > '{}/claude'\nexit 2\n",
                out.display()
            ),
        );

        let exit = renew_with(
            &renewal(&primary, InstanceKind::Primary, &neutral),
            &lookup(&bin, None),
            RENEW_TIMEOUT,
        )
        .unwrap();

        assert_eq!(exit.exit_code, Some(2));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(seen.trim(), format!("unset|{}", home.path().display()));
    }

    #[cfg(unix)]
    #[test]
    fn a_renewal_that_hangs_is_stopped_and_times_out() {
        let (_home, bin, dir, neutral, _out) = renewal_home();
        write_program(&bin.join("claude"), "#!/bin/sh\nexec sleep 30\n");

        let started = Instant::now();
        let error = renew_with(
            &renewal(&dir, InstanceKind::Linked, &neutral),
            &lookup(&bin, None),
            Duration::from_millis(300),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_renewal_runs_an_npm_claude_cmd_in_the_instance_from_the_neutral_folder() {
        let (_home, bin, dir, neutral, out) = renewal_home();
        fs::write(
            bin.join("claude.cmd"),
            format!(
                "@echo off\r\n>\"{}\\claude\" echo %CLAUDE_CONFIG_DIR%^|%CD%^|%*\r\n\
                 echo No MCP server named x. 1>&2\r\nexit /b 1\r\n",
                out.display()
            ),
        )
        .unwrap();

        let exit = renew_with(
            &renewal(&dir, InstanceKind::Linked, &neutral),
            &lookup(&bin, None),
            RENEW_TIMEOUT,
        )
        .unwrap();

        assert_eq!(exit.exit_code, Some(0));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(
            seen.trim().to_lowercase(),
            format!(
                "{}|{}|mcp get mccboard-renewal-no-such-server",
                dir.display(),
                neutral.display()
            )
            .to_lowercase()
        );
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_renewal_that_hangs_is_stopped_and_times_out() {
        let (_home, bin, dir, neutral, _out) = renewal_home();
        fs::write(
            bin.join("claude.cmd"),
            // Short: killing cmd.exe leaves ping running, in the temporary folder.
            "@echo off\r\nping -n 5 127.0.0.1 >nul\r\n",
        )
        .unwrap();

        let started = Instant::now();
        let error = renew_with(
            &renewal(&dir, InstanceKind::Linked, &neutral),
            &lookup(&bin, None),
            Duration::from_millis(500),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(4));
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
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}/term'\nshift\nexec \"$@\"\n",
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

    /// A desktop launched from its own menu often has no `$TERMINAL` and no
    /// `x-terminal-emulator` (Arch, Fedora): the first terminal mccboard knows is used, with
    /// the arguments it takes before the program.
    #[cfg(unix)]
    #[test]
    fn on_linux_the_first_known_terminal_on_the_path_is_used_with_its_own_arguments() {
        let bin = tempfile::tempdir().unwrap();
        for name in ["xterm", "gnome-terminal", "kitty"] {
            write_program(&bin.path().join(name), "#!/bin/sh\nexit 0\n");
        }

        let terminal = lookup(bin.path(), None).terminal().unwrap();
        assert_eq!(terminal.program, bin.path().join("kitty"));
        assert!(terminal.before_program.is_empty());

        fs::remove_file(bin.path().join("kitty")).unwrap();
        let terminal = lookup(bin.path(), None).terminal().unwrap();
        assert_eq!(terminal.program, bin.path().join("gnome-terminal"));
        assert_eq!(terminal.before_program, ["--wait", "--"]);
    }

    #[cfg(unix)]
    #[test]
    fn on_linux_terminal_and_x_terminal_emulator_come_before_the_known_terminals() {
        let bin = tempfile::tempdir().unwrap();
        for name in ["kitty", "x-terminal-emulator", "gnome-terminal", "my-term"] {
            write_program(&bin.path().join(name), "#!/bin/sh\nexit 0\n");
        }

        // `$TERMINAL` first, with its own arguments when mccboard knows it.
        let terminal = lookup(bin.path(), Some("gnome-terminal"))
            .terminal()
            .unwrap();
        assert_eq!(terminal.program, bin.path().join("gnome-terminal"));
        assert_eq!(terminal.before_program, ["--wait", "--"]);

        // One it doesn't know, as a path, takes `-e`.
        let my_term = bin.path().join("my-term");
        let terminal = lookup(bin.path(), my_term.to_str()).terminal().unwrap();
        assert_eq!(terminal.program, my_term);
        assert_eq!(terminal.before_program, ["-e"]);

        // Then Debian's choice, which takes `-e` whatever terminal it is.
        let terminal = lookup(bin.path(), None).terminal().unwrap();
        assert_eq!(terminal.program, bin.path().join("x-terminal-emulator"));
        assert_eq!(terminal.before_program, ["-e"]);
    }

    #[cfg(unix)]
    #[test]
    fn on_linux_a_terminal_without_dash_e_gets_the_program_alone() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        let out = home.path().join("out");
        for path in [&bin, &dir, &out] {
            fs::create_dir_all(path).unwrap();
        }
        // kitty runs the program it is given, with no `-e` before it.
        write_program(
            &bin.join("kitty"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}/term'\nexec \"$@\"\n",
                out.display()
            ),
        );
        write_program(&bin.join("claude"), "#!/bin/sh\nexit 4\n");

        let exit = login_with(
            &invocation(&dir, LoginTerminal::NewWindow),
            &lookup(&bin, None),
        )
        .unwrap();

        assert_eq!(exit.exit_code, Some(4));
        let term = fs::read_to_string(out.join("term")).unwrap();
        assert_eq!(
            term.trim(),
            format!("{} auth login --claudeai", bin.join("claude").display())
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
