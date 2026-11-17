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
//!
//! Outside any instance, `claude --version` runs hidden too, and Claude Code's official
//! installer (`curl … | bash`) runs in the current terminal, as the login does.
//!
//! The login, the background runs and the renewal find `claude` on the PATH, else in
//! `.local/bin` of the core's home, as [`crate::Core::claude_code`] does. The official
//! installer puts it there before the PATH has that folder, and the setup logs in right after
//! installing it.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::instance::InstanceKind;
use crate::{
    ClaudeInvocation, ClaudeRun, InstallerExit, InstallerInvocation, LoginExit, LoginInvocation,
    LoginTerminal, OsKind, RenewalExit, RenewalInvocation,
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
    let claude = lookup.claude(&invocation.home).ok_or_else(not_found)?;
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
    output(
        &mut command,
        TIMEOUT,
        "Claude Code didn't finish within 10 minutes",
    )
}

/// Starts `command`, whose output is piped, and waits for it to exit, keeping what it printed.
/// When it runs longer than `timeout` it is stopped, and the error says `too_slow`.
fn output(command: &mut Command, timeout: Duration, too_slow: &str) -> io::Result<ClaudeRun> {
    let mut child = command.spawn()?;

    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, too_slow));
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

/// The program `name` on the PATH, found as the login and background runs look for `claude`
/// there before trying `.local/bin`.
pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    Lookup::from_env().find(name)
}

/// `claude --version` answers at once. A few seconds cover a slow disk.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `<program> --version` in the background and keeps what it printed.
pub(crate) fn version(program: &Path) -> io::Result<ClaudeRun> {
    version_with(program, VERSION_TIMEOUT)
}

fn version_with(program: &Path, timeout: Duration) -> io::Result<ClaudeRun> {
    let mut command = crate::background::command(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    output(
        &mut command,
        timeout,
        "Claude Code didn't print its version in time",
    )
}

/// What `curl` gets in the official installer's command: fail on an HTTP error, print only
/// errors, follow redirects, and the address of the installer script. The command shown to
/// users ([`crate::CLAUDE_CODE_INSTALL_COMMAND`]) is the same.
const INSTALLER_DOWNLOAD_ARGS: &[&str] = &["-fsSL", "https://claude.ai/install.sh"];

/// Runs `curl … | bash` as two children joined by a pipe, and keeps each exit code: `curl`
/// failing leaves `bash` an empty script, which it runs with success. Both share mccboard's
/// terminal. `bash`'s output goes to mccboard's standard error, as the login's does.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn install(invocation: &InstallerInvocation) -> io::Result<InstallerExit> {
    let mut curl = Command::new(&invocation.curl)
        .args(INSTALLER_DOWNLOAD_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let script = curl
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("curl's output wasn't piped"))?;
    let mut command = Command::new(&invocation.bash);
    command
        .stdin(script)
        .stdout(io::stderr())
        .env(HOME_VAR, &invocation.home);
    let bash = command.spawn();
    // The command held the pipe's other end. Once it is dropped the pipe is bash's alone, and a
    // bash that stops early can't leave curl writing into a pipe nobody reads.
    drop(command);
    let installed = bash.and_then(|mut bash| bash.wait());
    if installed.is_err() {
        let _ = curl.kill();
    }
    let downloaded = curl.wait()?;
    Ok(InstallerExit {
        download_exit_code: downloaded.code(),
        exit_code: installed?.code(),
    })
}

/// Claude Code's official login, for claude.ai subscriptions.
const LOGIN_ARGS: &[&str] = &["auth", "login", "--claudeai"];

/// Opens `claude auth login` in the instance and waits for it to end. Its credentials and
/// output stay between Claude Code and the user.
pub(crate) fn login(login: &LoginInvocation) -> io::Result<LoginExit> {
    login_with(login, &Lookup::from_env())
}

fn not_found() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "claude is neither on the PATH nor in ~/.local/bin",
    )
}

/// Where Claude Code's official installer puts it in `home`: `.local/bin/claude`, or
/// `.local/bin/claude.exe` on Windows.
pub(crate) fn installed_claude(home: &Path, os: OsKind) -> PathBuf {
    let name = match os {
        OsKind::Windows => "claude.exe",
        OsKind::Linux => "claude",
    };
    home.join(".local").join("bin").join(name)
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

    /// Claude Code for a core working in `home`: `claude` on the PATH, else the one its
    /// official installer put in `.local/bin` of `home` before the PATH had that folder.
    fn claude(&self, home: &Path) -> Option<PathBuf> {
        self.find("claude").or_else(|| {
            let os = if cfg!(windows) {
                OsKind::Windows
            } else {
                OsKind::Linux
            };
            let installed = installed_claude(home, os);
            is_program(&installed).then_some(installed)
        })
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

/// Whether `path` is a file that can be run: on Unix, one with an execute bit.
#[cfg(unix)]
pub(crate) fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether `path` is a file that can be run: on Windows, any file.
#[cfg(not(unix))]
pub(crate) fn is_program(path: &Path) -> bool {
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
    let claude = lookup.claude(&renewal.home).ok_or_else(not_found)?;
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
    let working_dir = login_dir(login).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", login.config_dir.display()),
        )
    })?;
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
    let claude = lookup.claude(&login.home).ok_or_else(not_found)?;
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
    let status = command.current_dir(working_dir).status()?;
    Ok(LoginExit {
        exit_code: status.code(),
    })
}

/// Where the login runs: the instance's folder, or the home for the primary instance whose
/// folder isn't there yet, since Claude Code makes it on its first login. `None` when neither
/// exists: a linked instance's folder is never missing at a login.
fn login_dir(login: &LoginInvocation) -> Option<&Path> {
    if login.config_dir.is_dir() {
        return Some(&login.config_dir);
    }
    match login.kind {
        InstanceKind::Primary if !login.config_dir.exists() => {
            login.config_dir.parent().filter(|home| home.is_dir())
        }
        _ => None,
    }
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

    /// The home of a core whose `.local/bin` has no Claude Code.
    const NO_LOCAL_CLAUDE: &str = "/nowhere";

    /// Where the official installer puts Claude Code in `home`, with its folder created.
    fn local_claude(home: &Path) -> PathBuf {
        let os = if cfg!(windows) {
            crate::OsKind::Windows
        } else {
            crate::OsKind::Linux
        };
        let program = installed_claude(home, os);
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        program
    }

    #[test]
    fn claude_only_in_local_bin_of_the_home_is_found_there() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let local = local_claude(home.path());
        #[cfg(unix)]
        write_program(&local, "#!/bin/sh\nexit 0\n");
        #[cfg(not(unix))]
        fs::write(&local, "fake claude.exe").unwrap();

        assert_eq!(lookup(&bin, None).claude(home.path()), Some(local));
        // Other programs are looked for on the PATH only.
        assert_eq!(lookup(&bin, None).find("claude"), None);
        // Another home's `.local/bin` is not this one.
        assert_eq!(lookup(&bin, None).claude(Path::new(NO_LOCAL_CLAUDE)), None);
    }

    #[cfg(unix)]
    #[test]
    fn claude_on_the_path_comes_before_the_one_in_local_bin() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        write_program(&bin.join("claude"), "#!/bin/sh\nexit 0\n");
        write_program(&local_claude(home.path()), "#!/bin/sh\nexit 0\n");

        assert_eq!(
            lookup(&bin, None).claude(home.path()),
            Some(bin.join("claude"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_in_local_bin_that_cannot_run_is_not_claude() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(local_claude(home.path()), "not a program").unwrap();

        assert_eq!(lookup(&bin, None).claude(home.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_run_uses_claude_from_local_bin_when_the_path_lacks_it() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&dir).unwrap();
        write_program(
            &local_claude(home.path()),
            "#!/bin/sh\nprintf '%s\\n' \"local|$*\"\nexit 3\n",
        );

        let invocation = ClaudeInvocation {
            home: home.path().to_path_buf(),
            ..claude_invocation(&dir)
        };
        let run = run_with(&invocation, &lookup(&bin, None)).unwrap();

        assert_eq!(run.exit_code, Some(3));
        assert_eq!(run.stdout.trim(), "local|plugin list");
    }

    #[cfg(unix)]
    #[test]
    fn a_login_uses_claude_from_local_bin_when_the_path_lacks_it() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let dir = home.path().join(".claude-b");
        let out = home.path().join("out");
        for folder in [&bin, &dir, &out] {
            fs::create_dir_all(folder).unwrap();
        }
        // A terminal emulator that runs the program after `-e`.
        write_program(&bin.join("my-term"), "#!/bin/sh\nshift\nexec \"$@\"\n");
        let local = local_claude(home.path());
        write_program(
            &local,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$0|$*\" > '{}/claude'\nexit 0\n",
                out.display()
            ),
        );

        let login = LoginInvocation {
            home: home.path().to_path_buf(),
            ..invocation(&dir, LoginTerminal::NewWindow)
        };
        let exit = login_with(&login, &lookup(&bin, Some("my-term"))).unwrap();

        assert_eq!(exit.exit_code, Some(0));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(
            seen.trim(),
            format!("{}|auth login --claudeai", local.display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_renewal_uses_claude_from_local_bin_when_the_path_lacks_it() {
        let (home, bin, dir, neutral, out) = renewal_home();
        write_program(
            &local_claude(home.path()),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}/claude'\nexit 1\n",
                out.display()
            ),
        );

        let invocation = RenewalInvocation {
            home: home.path().to_path_buf(),
            ..renewal(&dir, InstanceKind::Linked, &neutral)
        };
        let exit = renew_with(&invocation, &lookup(&bin, None), RENEW_TIMEOUT).unwrap();

        assert_eq!(exit.exit_code, Some(0));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(seen.trim(), RENEW_ARGS.join(" "));
    }

    fn invocation(config_dir: &Path, terminal: LoginTerminal) -> LoginInvocation {
        LoginInvocation {
            config_dir: config_dir.to_path_buf(),
            kind: InstanceKind::Linked,
            terminal,
            home: PathBuf::from(NO_LOCAL_CLAUDE),
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
        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn a_primary_instance_without_a_home_is_refused_too() {
        let home = tempfile::tempdir().unwrap();
        let primary = home.path().join("gone").join(".claude");
        let login = LoginInvocation {
            kind: InstanceKind::Primary,
            ..invocation(&primary, LoginTerminal::NewWindow)
        };

        let error = login_with(&login, &lookup(home.path(), None)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    }

    #[cfg(unix)]
    #[test]
    fn a_first_login_of_the_primary_instance_runs_in_the_home_where_claude_code_makes_its_folder() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let out = home.path().join("out");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&out).unwrap();
        let primary = home.path().join(".claude");
        write_program(&bin.join("my-term"), "#!/bin/sh\nshift\nexec \"$@\"\n");
        // Claude Code makes its folder on its first login.
        write_program(
            &bin.join("claude"),
            &format!(
                "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$PWD\" \"$HOME\" \
                 \"${{CLAUDE_CONFIG_DIR-unset}}\" > '{}/claude'\nmkdir \"$HOME/.claude\"\n",
                out.display()
            ),
        );
        let login = LoginInvocation {
            kind: InstanceKind::Primary,
            ..invocation(&primary, LoginTerminal::NewWindow)
        };

        let exit = login_with(&login, &lookup(&bin, Some("my-term"))).unwrap();

        assert_eq!(exit.exit_code, Some(0));
        let seen = fs::read_to_string(out.join("claude")).unwrap();
        let home = home.path().display();
        assert_eq!(seen.trim(), format!("{home}|{home}|unset"));
        assert!(primary.is_dir());
    }

    fn claude_invocation(config_dir: &Path) -> ClaudeInvocation {
        ClaudeInvocation {
            config_dir: config_dir.to_path_buf(),
            args: vec!["plugin".into(), "list".into()],
            home: PathBuf::from(NO_LOCAL_CLAUDE),
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
            home: PathBuf::from(NO_LOCAL_CLAUDE),
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

    /// A home with a `bin` folder, the temporary PATH, and an `out` folder where fake
    /// programs write what they saw.
    fn bin_home() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let [bin, out] = ["bin", "out"].map(|name| {
            let path = home.path().join(name);
            fs::create_dir_all(&path).unwrap();
            path
        });
        (home, bin, out)
    }

    #[cfg(unix)]
    #[test]
    fn a_version_run_starts_claude_found_on_the_path_with_version_and_keeps_its_output() {
        let (_home, bin, out) = bin_home();
        write_program(
            &bin.join("claude"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}/claude'\n\
                 echo '2.1.301 (Claude Code)'\necho 'a warning' >&2\n",
                out.display()
            ),
        );
        let claude = lookup(&bin, None).find("claude").unwrap();

        let run = version_with(&claude, VERSION_TIMEOUT).unwrap();

        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.stdout, "2.1.301 (Claude Code)\n");
        assert_eq!(run.stderr, "a warning\n");
        let args = fs::read_to_string(out.join("claude")).unwrap();
        assert_eq!(args, "--version\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_version_run_that_hangs_is_stopped_and_times_out() {
        let (_home, bin, _out) = bin_home();
        write_program(&bin.join("claude"), "#!/bin/sh\nexec sleep 30\n");

        let started = Instant::now();
        let error = version_with(&bin.join("claude"), Duration::from_millis(300)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_version_run_starts_an_npm_claude_cmd() {
        let (_home, bin, _out) = bin_home();
        fs::write(
            bin.join("claude.cmd"),
            "@echo off\r\necho 2.1.301 (Claude Code) %*\r\n",
        )
        .unwrap();
        let claude = lookup(&bin, None).find("claude").unwrap();

        let run = version_with(&claude, VERSION_TIMEOUT).unwrap();

        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.stdout.trim(), "2.1.301 (Claude Code) --version");
    }

    #[test]
    fn the_installer_runs_the_command_users_are_shown() {
        assert_eq!(
            format!("curl {} | bash", INSTALLER_DOWNLOAD_ARGS.join(" ")),
            crate::CLAUDE_CODE_INSTALL_COMMAND
        );
    }

    /// Fake `curl` and `bash` on the temporary PATH: `curl` records its arguments and runs
    /// `curl_body`, and `bash` records its `HOME`, then runs `bash_body`.
    #[cfg(unix)]
    fn installer(
        bin: &Path,
        out: &Path,
        home: &Path,
        curl_body: &str,
        bash_body: &str,
    ) -> InstallerInvocation {
        write_program(
            &bin.join("curl"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}/curl'\n{curl_body}\n",
                out.display()
            ),
        );
        write_program(
            &bin.join("bash"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$HOME\" > '{out}/home'\n{bash_body}\n",
                out = out.display()
            ),
        );
        let lookup = lookup(bin, None);
        InstallerInvocation {
            curl: lookup.find("curl").unwrap(),
            bash: lookup.find("bash").unwrap(),
            home: home.to_path_buf(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_installer_pipes_the_official_script_from_curl_into_bash_in_the_home() {
        let (home, bin, out) = bin_home();
        let invocation = installer(
            &bin,
            &out,
            home.path(),
            "printf 'echo installing\\n'",
            &format!("cat > '{}/script'", out.display()),
        );

        let exit = install(&invocation).unwrap();

        assert_eq!(
            exit,
            InstallerExit {
                download_exit_code: Some(0),
                exit_code: Some(0),
            }
        );
        let curl = fs::read_to_string(out.join("curl")).unwrap();
        assert_eq!(curl, "-fsSL https://claude.ai/install.sh\n");
        let script = fs::read_to_string(out.join("script")).unwrap();
        assert_eq!(script, "echo installing\n");
        let seen_home = fs::read_to_string(out.join("home")).unwrap();
        assert_eq!(seen_home.trim(), home.path().display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_installer_reports_bashs_exit_code() {
        let (home, bin, out) = bin_home();
        let invocation = installer(
            &bin,
            &out,
            home.path(),
            "printf 'exit 3\\n'",
            "cat > /dev/null\nexit 3",
        );

        let exit = install(&invocation).unwrap();

        assert_eq!(exit.exit_code, Some(3));
        assert_eq!(exit.download_exit_code, Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_download_reports_curls_exit_code_while_bash_ends_well() {
        let (home, bin, out) = bin_home();
        // `curl -f` on an HTTP error: nothing on its output, exit code 22.
        let invocation = installer(&bin, &out, home.path(), "exit 22", "cat > /dev/null");

        let exit = install(&invocation).unwrap();

        assert_eq!(
            exit,
            InstallerExit {
                download_exit_code: Some(22),
                exit_code: Some(0),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_bash_that_stops_early_does_not_leave_curl_writing_forever() {
        let (home, bin, out) = bin_home();
        // Far more than a pipe holds, to a bash that reads none of it.
        let invocation = installer(
            &bin,
            &out,
            home.path(),
            "i=0\nwhile [ $i -lt 20000 ]; do printf 'echo %0100d\\n' $i || exit 23; i=$((i+1)); done",
            "exit 4",
        );

        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || sender.send(install(&invocation).unwrap()));
        let exit = receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("the installer still waits for curl");

        assert_eq!(exit.exit_code, Some(4));
        assert_ne!(exit.download_exit_code, Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn an_installer_whose_bash_cannot_start_fails_and_stops_curl() {
        let (home, bin, out) = bin_home();
        let mut invocation = installer(&bin, &out, home.path(), "exec sleep 30", "exit 0");
        invocation.bash = bin.join("no-such-bash");

        let started = Instant::now();
        let error = install(&invocation).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_the_system_installer_is_unsupported() {
        use crate::Environment;
        let home = tempfile::tempdir().unwrap();
        let invocation = InstallerInvocation {
            curl: home.path().join("curl.exe"),
            bash: home.path().join("bash.exe"),
            home: home.path().to_path_buf(),
        };

        let error = crate::SystemEnvironment
            .install_claude_code(&invocation)
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }
}
