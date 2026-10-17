use std::io;
use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use crate::desktop_identity::Shortcut;
use crate::desktop_launch::{DesktopLaunch, DesktopPackage};
use crate::desktop_process::ProcessListing;
use crate::instance::InstanceKind;
use crate::session_guard::ClaudeProcess;
use crate::{AccessToken, Image};

/// The port through which the core reaches everything outside the filesystem.
///
/// One real implementation per OS ([`SystemEnvironment`]) and one fake for tests
/// (`testing::FakeEnvironment`).
pub trait Environment: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;

    /// Calls the usage endpoint (`GET https://api.anthropic.com/api/oauth/usage`) with
    /// `token`. The core calls it at most once per account per rate-limit interval, and never
    /// with an expired token. An implementation must not log, store or echo the token.
    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply;

    /// The operating system of the local machine. It decides, among other things, what a
    /// launcher is: a shell alias on Linux, a program on the user PATH on Windows.
    fn os(&self) -> OsKind;

    /// The user's login shell (`$SHELL`), which picks the rc file launchers go in on Linux.
    fn login_shell(&self) -> Option<PathBuf>;

    /// Whether `dir` is on the user PATH that new terminals get (on Windows, the `Path` value
    /// under `HKCU\Environment`).
    fn user_path_contains(&self, dir: &Path) -> io::Result<bool>;

    /// Appends `dir` to the user PATH that new terminals get, and tells running programs that
    /// it changed. Terminals already open keep their PATH.
    fn add_to_user_path(&self, dir: &Path) -> io::Result<()>;

    /// The shim mccboard ships, the program a Windows launcher is a copy of (`claude-b.exe`):
    /// its bytes. It ships next to mccboard's own programs, the CLI and the app. Errors when
    /// it isn't there or can't be read; the message names where it was looked for.
    fn launcher_shim(&self) -> io::Result<Vec<u8>>;

    /// The CLI mccboard ships (`mccboard.exe`), which on Windows a launcher sync and the
    /// app's start copy into the bin folder: where it is. It ships next to the app, as the
    /// shim does. Errors when it isn't there; the message names where it was looked for.
    fn shipped_cli(&self) -> io::Result<PathBuf>;

    /// Every running Claude Code process on this machine, each with the config directory it
    /// uses. The core matches them to instances (the session guard).
    fn claude_processes(&self) -> Vec<ClaudeProcess>;

    /// Every running process named `Claude` (`Claude.exe`) on this machine that mccboard can
    /// see, with its program, parent and command line. The core tells which are the desktop
    /// app (main processes and helpers) and which desktop profile each runs: a profile's data
    /// folder is never deleted while one uses it.
    fn desktop_processes(&self) -> Vec<ProcessListing>;

    /// The installed MSIX packages of the given package families, for the current user, each
    /// with its install folder (Windows' packaging API). Empty when none is installed. Errors
    /// when the API fails, and with `Unsupported` off Windows.
    fn desktop_app_packages(&self, families: &[String]) -> io::Result<Vec<DesktopPackage>>;

    /// Starts the desktop app as `launch` says, with no input or output and no console window,
    /// and returns its process id without waiting for it.
    fn launch_desktop_app(&self, launch: &DesktopLaunch) -> io::Result<u32>;

    /// The icon of the program `program` (the desktop app's `Claude.exe`) at its largest size:
    /// the logo a desktop profile's icon is drawn from when its package has none. Errors when
    /// it has no icon or can't be read, and with `Unsupported` off Windows.
    fn program_icon(&self, program: &Path) -> io::Result<Image>;

    /// The current user's Start-menu Programs folder, where desktop profiles' shortcuts go
    /// (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`), for a core working in `home`.
    /// `None` when `home` isn't the current user's (`mccboard --home`, tests): shortcuts made
    /// there would never show in the Start menu, and the user's own are never touched.
    /// `Unsupported` off Windows.
    fn start_menu_programs(&self, home: &Path) -> io::Result<Option<PathBuf>>;

    /// mccboard's app, the program a desktop profile's shortcut starts: the running program
    /// when it is the app, else the app next to it (the CLI's folder). Errors when it isn't
    /// there.
    fn mccboard_app(&self) -> io::Result<PathBuf>;

    /// Writes the shell link `shortcut.path` as `shortcut` says, its AppUserModelID included,
    /// replacing a file already there. `Unsupported` off Windows.
    fn write_shortcut(&self, shortcut: &Shortcut) -> io::Result<()>;

    /// Whether file links (symlinks to files) can be created without elevation: on Windows,
    /// whether Developer Mode is on (ADR 0006); elsewhere always.
    fn file_links_allowed(&self) -> bool;

    /// Runs Claude Code (`claude <args>`) in the linked instance `invocation.config_dir`:
    /// with `CLAUDE_CONFIG_DIR` set to it, without the variables that would override its own
    /// login (`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`), with no
    /// input, and waits for it to exit. Never for the primary instance, whose global config
    /// lives outside its folder. Errors when `claude` can't be started (`NotFound` when it
    /// isn't installed) or doesn't finish in time.
    fn run_claude(&self, invocation: &ClaudeInvocation) -> io::Result<ClaudeRun>;

    /// Opens Claude Code's official login (`claude auth login`) in the instance
    /// `login.config_dir`, in the terminal `login.terminal` names, and waits for it to end.
    /// The user logs in in the browser; Claude Code writes the credentials and `.claude.json`
    /// itself, and mccboard never sees them. For a linked instance `CLAUDE_CONFIG_DIR` is set
    /// to its folder; for the primary instance it is unset and the home is the folder's
    /// parent. The variables that would override the instance's own login are removed.
    ///
    /// Errors, before anything starts, with `NotFound` only when `claude` itself isn't found
    /// on the PATH; with `Unsupported` when there is no such terminal (no terminal for the
    /// current one, no terminal emulator for a new window on Linux); with another kind when
    /// the instance's folder is missing or the program can't be started.
    fn open_login(&self, login: &LoginInvocation) -> io::Result<LoginExit>;

    /// Runs `claude mcp get` with a missing name in the instance `renewal.config_dir`, so that
    /// Claude Code renews the instance's expired token on its own and rewrites its own
    /// credentials (ADR 0002): it makes no model call and starts no session window. mccboard never refreshes
    /// the token itself.
    ///
    /// Like the login, a linked instance gets `CLAUDE_CONFIG_DIR`, and the primary instance
    /// none, with the home set to its folder's parent; the variables that would override the
    /// instance's own login are removed. Unlike it, the command runs in the background: hidden
    /// (no console window), with no input, in `renewal.working_dir`, and its output is
    /// discarded unread, since it names the account. Waits for it to exit.
    ///
    /// Errors with `NotFound` only when `claude` isn't found on the PATH, with `TimedOut` when
    /// it doesn't finish within a minute (it is then stopped), with another kind when it can't
    /// be started.
    fn renew_token(&self, renewal: &RenewalInvocation) -> io::Result<RenewalExit>;
}

/// Claude Code's own renewal, to run in an instance whose token has expired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenewalInvocation {
    /// The instance's config directory.
    pub config_dir: PathBuf,
    /// Primary (Claude Code's default folder, no `CLAUDE_CONFIG_DIR`) or linked.
    pub kind: InstanceKind,
    /// A folder of mccboard's own to run in (`~/.mccboard/state/renewal/`), so no project's
    /// settings, hooks or `CLAUDE.md` apply.
    pub working_dir: PathBuf,
}

/// How the renewal command ended. The core doesn't trust it: it reads the token again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenewalExit {
    /// The exit code; `None` when it was ended by a signal or can't be known.
    pub exit_code: Option<i32>,
}

/// Claude Code's login, to open in an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoginInvocation {
    /// The instance's config directory.
    pub config_dir: PathBuf,
    /// Primary (Claude Code's default folder, no `CLAUDE_CONFIG_DIR`) or linked.
    pub kind: InstanceKind,
    pub terminal: LoginTerminal,
}

/// Where the login runs. The front end chooses: the app, which has no terminal, a new
/// window; the CLI its own terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginTerminal {
    /// A new terminal window, which closes when the login ends: a console of its own on
    /// Windows; on Linux a terminal emulator (`$TERMINAL`, else `x-terminal-emulator`, else
    /// `xterm`).
    NewWindow,
    /// The terminal mccboard's CLI runs in. Claude Code's output goes to mccboard's standard
    /// error, so mccboard's standard output carries only its own report (`--json`).
    CurrentTerminal,
}

/// How the login ended. The core doesn't trust it: it observes the instance afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LoginExit {
    /// The exit code; `None` when it was ended by a signal or can't be known.
    pub exit_code: Option<i32>,
}

/// A `claude` command to run in an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeInvocation {
    /// The instance's config directory.
    pub config_dir: PathBuf,
    /// The arguments after `claude` (`plugin install review@tools --json`).
    pub args: Vec<String>,
}

/// How a `claude` command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeRun {
    /// The exit code; `None` when it was ended by a signal.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ClaudeRun {
    /// A run that exited with 0 and printed `stdout`.
    pub fn ok(stdout: &str) -> Self {
        Self::exit(0, stdout, "")
    }

    /// A run that exited with `code`.
    pub fn exit(code: i32, stdout: &str, stderr: &str) -> Self {
        Self {
            exit_code: Some(code),
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// What the usage endpoint answered. The core reads it; the port only carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointReply {
    /// An HTTP reply, whatever its status. `retry_after` is the `Retry-After` header in
    /// seconds, when there is one.
    Http {
        status: u16,
        retry_after: Option<SignedDuration>,
        body: String,
    },
    /// No reply: network, proxy, TLS or timeout. `reason` is a short fixed description, never
    /// the HTTP client's own message.
    Unreachable { reason: String },
}

/// The operating system family of a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OsKind {
    Linux,
    Windows,
}

/// The real environment of the machine mccboard runs on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl SystemEnvironment {
    /// Tells the environment that the running program is mccboard's app, `program`
    /// (`std::env::current_exe()`): the program desktop profiles' shortcuts start. The app
    /// calls it once when it starts; the CLI never does, and finds the app next to itself.
    pub fn set_app_program(program: PathBuf) {
        crate::system::set_app_program(program);
    }
}

impl Environment for SystemEnvironment {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply {
        crate::usage::call_endpoint(token)
    }

    fn os(&self) -> OsKind {
        if cfg!(windows) {
            OsKind::Windows
        } else {
            OsKind::Linux
        }
    }

    fn login_shell(&self) -> Option<PathBuf> {
        std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
    }

    #[cfg(windows)]
    fn user_path_contains(&self, dir: &Path) -> io::Result<bool> {
        crate::windows_user_path::contains(dir)
    }

    #[cfg(not(windows))]
    fn user_path_contains(&self, _dir: &Path) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the user PATH is managed on Windows only",
        ))
    }

    /// Only ever adds mccboard's own bin folder in the real home (`~/.mccboard/bin`): a core
    /// working in another home (`mccboard --home`, tests) never changes the user's PATH.
    #[cfg(windows)]
    fn add_to_user_path(&self, dir: &Path) -> io::Result<()> {
        let own = std::env::home_dir().map(|home| crate::Paths::new(&home).bin_dir());
        if !own.is_some_and(|own| crate::paths::same(&own, dir)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only mccboard's bin folder in your home is added to the user PATH",
            ));
        }
        crate::windows_user_path::add(dir)
    }

    #[cfg(not(windows))]
    fn add_to_user_path(&self, _dir: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the user PATH is managed on Windows only",
        ))
    }

    fn launcher_shim(&self) -> io::Result<Vec<u8>> {
        crate::system::launcher_shim()
    }

    fn shipped_cli(&self) -> io::Result<PathBuf> {
        crate::system::shipped_cli()
    }

    fn claude_processes(&self) -> Vec<ClaudeProcess> {
        crate::system::claude_processes()
    }

    fn desktop_processes(&self) -> Vec<ProcessListing> {
        crate::system::desktop_processes()
    }

    #[cfg(windows)]
    fn desktop_app_packages(&self, families: &[String]) -> io::Result<Vec<DesktopPackage>> {
        crate::windows_desktop_app::packages(families)
    }

    #[cfg(not(windows))]
    fn desktop_app_packages(&self, _families: &[String]) -> io::Result<Vec<DesktopPackage>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the Claude desktop app is looked up on Windows only",
        ))
    }

    fn launch_desktop_app(&self, launch: &DesktopLaunch) -> io::Result<u32> {
        crate::system::launch_desktop_app(launch)
    }

    #[cfg(windows)]
    fn program_icon(&self, program: &Path) -> io::Result<Image> {
        crate::windows_shell::program_icon(program)
    }

    #[cfg(not(windows))]
    fn program_icon(&self, _program: &Path) -> io::Result<Image> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "program icons are read on Windows only",
        ))
    }

    /// Only for a core working in the user's own home: one working in another home
    /// (`mccboard --home`, tests) never writes in the user's Start menu.
    #[cfg(windows)]
    fn start_menu_programs(&self, home: &Path) -> io::Result<Option<PathBuf>> {
        let own = std::env::home_dir().is_some_and(|own| crate::paths::same(&own, home));
        if !own {
            return Ok(None);
        }
        crate::windows_shell::start_menu_programs().map(Some)
    }

    #[cfg(not(windows))]
    fn start_menu_programs(&self, _home: &Path) -> io::Result<Option<PathBuf>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the Start menu exists on Windows only",
        ))
    }

    fn mccboard_app(&self) -> io::Result<PathBuf> {
        crate::system::mccboard_app()
    }

    #[cfg(windows)]
    fn write_shortcut(&self, shortcut: &Shortcut) -> io::Result<()> {
        crate::windows_shell::write_shortcut(shortcut)
    }

    #[cfg(not(windows))]
    fn write_shortcut(&self, _shortcut: &Shortcut) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Start-menu shortcuts are written on Windows only",
        ))
    }

    fn file_links_allowed(&self) -> bool {
        crate::system::file_links_allowed()
    }

    fn run_claude(&self, invocation: &ClaudeInvocation) -> io::Result<ClaudeRun> {
        crate::claude_cli::run(invocation)
    }

    fn open_login(&self, login: &LoginInvocation) -> io::Result<LoginExit> {
        crate::claude_cli::login(login)
    }

    fn renew_token(&self, renewal: &RenewalInvocation) -> io::Result<RenewalExit> {
        crate::claude_cli::renew(renewal)
    }
}
