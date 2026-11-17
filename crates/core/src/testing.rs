//! Test support: the fake environment port. Enabled by the `testing` feature.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::{SignedDuration, Timestamp};

use serde_json::{Map, Value, json};

use crate::{
    AccessToken, ClaudeInvocation, ClaudeProcess, ClaudeRun, DesktopLaunch, DesktopPackage,
    EndpointReply, Environment, Image, InstallerExit, InstallerInvocation, InstanceKind, LoginExit,
    LoginInvocation, OsKind, ProcessListing, RenewalExit, RenewalInvocation, Shortcut,
};

/// The first pid a fake launch of the desktop app returns; each launch gets the next one.
pub const FIRST_FAKE_DESKTOP_PID: u32 = 4000;

/// The dummy access token [`FakeEnvironment::login_as`] writes, playing Claude Code. Nothing
/// mccboard produces may contain it.
pub const FAKE_LOGIN_TOKEN: &str = "sk-ant-oat01-FAKE-LOGIN-TOKEN-never-shown-by-mccboard";

/// mccboard's app as [`FakeEnvironment`] finds it unless told otherwise.
pub const FAKE_MCCBOARD_APP: &str = r"C:\Program Files\mccboard\mccboard.exe";

/// The launcher shim [`FakeEnvironment`] ships unless told otherwise.
pub const FAKE_LAUNCHER_SHIM: &[u8] = b"MZ fake mccboard-shim.exe, version 1";

/// What a fake `claude --version` prints unless told otherwise.
pub const FAKE_CLAUDE_VERSION: &str = "2.1.301 (Claude Code)";

/// A scriptable [`Environment`] for tests. Clones share state, so a test can keep a handle
/// after giving one to [`crate::Core::new`] and keep steering it.
///
/// It starts as a Linux machine with no login shell and an empty user PATH, whatever OS the
/// tests run on, shipping [`FAKE_LAUNCHER_SHIM`] as the launcher shim. Its PATH holds no
/// program: Claude Code, `curl` and `bash` are missing until a test puts them there.
#[derive(Debug, Clone)]
pub struct FakeEnvironment {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    now: Timestamp,
    usage_replies: VecDeque<EndpointReply>,
    /// One entry per usage endpoint call: a fingerprint of its token, never the token.
    usage_calls: Vec<u64>,
    os: OsKind,
    login_shell: Option<PathBuf>,
    user_path: Vec<PathBuf>,
    user_path_additions: usize,
    user_path_fails: bool,
    launcher_shim: Option<Vec<u8>>,
    shipped_cli: Option<PathBuf>,
    processes: Vec<ClaudeProcess>,
    desktop_processes: Vec<ProcessListing>,
    desktop_packages: Vec<DesktopPackage>,
    desktop_lookup_error: Option<String>,
    /// The families asked for, one list per lookup.
    desktop_lookups: Vec<Vec<String>>,
    desktop_launches: Vec<DesktopLaunch>,
    desktop_launch_handler: DesktopLaunchHandler,
    program_icon: Option<Image>,
    start_menu: Option<PathBuf>,
    other_home: bool,
    mccboard_app: Option<PathBuf>,
    shortcuts_written: Vec<Shortcut>,
    shortcut_error: Option<String>,
    developer_mode: bool,
    claude_invocations: Vec<ClaudeInvocation>,
    claude_handler: ClaudeHandler,
    login_invocations: Vec<LoginInvocation>,
    login_handler: LoginHandler,
    renewal_invocations: Vec<RenewalInvocation>,
    renewal_handler: RenewalHandler,
    /// The PATH: each program by name.
    programs: BTreeMap<String, PathBuf>,
    /// The program of each `--version` run.
    version_runs: Vec<PathBuf>,
    version_handler: VersionHandler,
    installer_invocations: Vec<InstallerInvocation>,
    installer_handler: InstallerHandler,
}

type HandlerFn = dyn Fn(&ClaudeInvocation) -> io::Result<ClaudeRun> + Send + Sync;

/// What a fake `claude` command does; by default it succeeds and prints nothing.
#[derive(Clone)]
struct ClaudeHandler(Arc<HandlerFn>);

type LoginFn = dyn Fn(&LoginInvocation) -> io::Result<LoginExit> + Send + Sync;

/// What a fake login does; by default the user closes it without logging in.
#[derive(Clone)]
struct LoginHandler(Arc<LoginFn>);

type RenewalFn = dyn Fn(&RenewalInvocation) -> io::Result<RenewalExit> + Send + Sync;

/// What a fake token renewal does; by default Claude Code exits 0 and renews nothing.
#[derive(Clone)]
struct RenewalHandler(Arc<RenewalFn>);

impl std::fmt::Debug for RenewalHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RenewalHandler")
    }
}

type VersionFn = dyn Fn(&Path) -> io::Result<ClaudeRun> + Send + Sync;

/// What a fake `claude --version` does. By default it prints [`FAKE_CLAUDE_VERSION`].
#[derive(Clone)]
struct VersionHandler(Arc<VersionFn>);

impl std::fmt::Debug for VersionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VersionHandler")
    }
}

type InstallerFn = dyn Fn(&InstallerInvocation) -> io::Result<InstallerExit> + Send + Sync;

/// What Claude Code's fake installer does. By default it exits 0 and installs nothing.
#[derive(Clone)]
struct InstallerHandler(Arc<InstallerFn>);

impl std::fmt::Debug for InstallerHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InstallerHandler")
    }
}

type DesktopLaunchFn = dyn Fn(&DesktopLaunch) -> io::Result<u32> + Send + Sync;

/// What a fake launch of the desktop app does; by default it starts nothing and returns the
/// next pid from [`FIRST_FAKE_DESKTOP_PID`].
#[derive(Clone)]
struct DesktopLaunchHandler(Option<Arc<DesktopLaunchFn>>);

impl std::fmt::Debug for DesktopLaunchHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DesktopLaunchHandler")
    }
}

impl std::fmt::Debug for LoginHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoginHandler")
    }
}

impl std::fmt::Debug for ClaudeHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClaudeHandler")
    }
}

impl FakeEnvironment {
    /// A fake whose clock reads `now` until moved, with no Claude Code process running and
    /// Developer Mode on.
    pub fn new(now: Timestamp) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                now,
                usage_replies: VecDeque::new(),
                usage_calls: Vec::new(),
                os: OsKind::Linux,
                login_shell: None,
                user_path: Vec::new(),
                user_path_additions: 0,
                user_path_fails: false,
                launcher_shim: Some(FAKE_LAUNCHER_SHIM.to_vec()),
                shipped_cli: None,
                processes: Vec::new(),
                desktop_processes: Vec::new(),
                desktop_packages: Vec::new(),
                desktop_lookup_error: None,
                desktop_lookups: Vec::new(),
                desktop_launches: Vec::new(),
                desktop_launch_handler: DesktopLaunchHandler(None),
                program_icon: None,
                start_menu: None,
                other_home: false,
                mccboard_app: Some(PathBuf::from(FAKE_MCCBOARD_APP)),
                shortcuts_written: Vec::new(),
                shortcut_error: None,
                developer_mode: true,
                claude_invocations: Vec::new(),
                claude_handler: ClaudeHandler(Arc::new(|_| Ok(ClaudeRun::ok("")))),
                login_invocations: Vec::new(),
                login_handler: LoginHandler(Arc::new(|_| Ok(LoginExit { exit_code: Some(1) }))),
                renewal_invocations: Vec::new(),
                renewal_handler: RenewalHandler(Arc::new(|_| {
                    Ok(RenewalExit { exit_code: Some(0) })
                })),
                programs: BTreeMap::new(),
                version_runs: Vec::new(),
                version_handler: VersionHandler(Arc::new(|_| {
                    Ok(ClaudeRun::ok(&format!("{FAKE_CLAUDE_VERSION}\n")))
                })),
                installer_invocations: Vec::new(),
                installer_handler: InstallerHandler(Arc::new(|_| {
                    Ok(InstallerExit {
                        download_exit_code: Some(0),
                        exit_code: Some(0),
                    })
                })),
            })),
        }
    }

    /// Sets the clock.
    pub fn set_now(&self, now: Timestamp) {
        self.state().now = now;
    }

    /// Moves the clock forward (or backward, with a negative duration).
    pub fn advance(&self, by: SignedDuration) {
        let mut state = self.state();
        state.now += by;
    }

    /// Queues the reply to the next usage endpoint call. With nothing queued, a call reads as
    /// unreachable.
    pub fn push_usage_reply(&self, reply: EndpointReply) {
        self.state().usage_replies.push_back(reply);
    }

    /// How many times the usage endpoint was called.
    pub fn usage_calls(&self) -> usize {
        self.state().usage_calls.len()
    }

    /// How many usage endpoint calls used the token `token`. The fake keeps fingerprints
    /// only, so the token never leaves its opaque type.
    pub fn usage_calls_with_token(&self, token: &str) -> usize {
        let fingerprint = crate::usage::fingerprint(token);
        self.state()
            .usage_calls
            .iter()
            .filter(|call| **call == fingerprint)
            .count()
    }

    /// Makes the fake machine run `os`.
    pub fn set_os(&self, os: OsKind) {
        self.state().os = os;
    }

    /// Sets the login shell (`$SHELL`), e.g. `/usr/bin/zsh`; `None` when unset.
    pub fn set_login_shell(&self, shell: Option<&str>) {
        self.state().login_shell = shell.map(PathBuf::from);
    }

    /// The fake user PATH, in order.
    pub fn user_path(&self) -> Vec<PathBuf> {
        self.state().user_path.clone()
    }

    /// How many times a folder was added to the user PATH.
    pub fn user_path_additions(&self) -> usize {
        self.state().user_path_additions
    }

    /// Makes reading and changing the user PATH fail, as a locked-down registry would.
    pub fn set_user_path_fails(&self, fails: bool) {
        self.state().user_path_fails = fails;
    }

    /// Replaces the running Claude Code processes.
    pub fn set_processes(&self, processes: Vec<ClaudeProcess>) {
        self.state().processes = processes;
    }

    /// Starts one more Claude Code process.
    pub fn add_process(&self, process: ClaudeProcess) {
        self.state().processes.push(process);
    }

    /// Replaces the running processes named `Claude`, as the OS lists them (the desktop app's
    /// and any other).
    pub fn set_desktop_processes(&self, processes: Vec<ProcessListing>) {
        self.state().desktop_processes = processes;
    }

    /// Installs the desktop app's MSIX packages: what every lookup finds, whatever families
    /// it asks for. None by default: the app isn't installed.
    pub fn set_desktop_app_packages(&self, packages: Vec<DesktopPackage>) {
        self.state().desktop_packages = packages;
    }

    /// Makes looking up the desktop app's packages fail with `message`, as when Windows'
    /// packaging API does; `None` undoes it.
    pub fn set_desktop_app_lookup_error(&self, message: Option<&str>) {
        self.state().desktop_lookup_error = message.map(str::to_owned);
    }

    /// The package families each lookup of the desktop app asked for, in order.
    pub fn desktop_app_lookups(&self) -> Vec<Vec<String>> {
        self.state().desktop_lookups.clone()
    }

    /// Scripts what launching the desktop app does: `handler` gets each launch and returns
    /// the pid started, or an error as when the program can't be started. It may look at the
    /// filesystem, to check what existed at launch.
    pub fn on_desktop_launch(
        &self,
        handler: impl Fn(&DesktopLaunch) -> io::Result<u32> + Send + Sync + 'static,
    ) {
        self.state().desktop_launch_handler = DesktopLaunchHandler(Some(Arc::new(handler)));
    }

    /// Every launch of the desktop app so far, in order.
    pub fn desktop_launches(&self) -> Vec<DesktopLaunch> {
        self.state().desktop_launches.clone()
    }

    /// Gives the desktop app's program an icon, the fallback logo when its package has none;
    /// none by default.
    pub fn set_program_icon(&self, icon: Option<Image>) {
        self.state().program_icon = icon;
    }

    /// Sets the Start menu's Programs folder; `None` (the default): it can't be found.
    pub fn set_start_menu(&self, folder: Option<&Path>) {
        self.state().start_menu = folder.map(Path::to_path_buf);
    }

    /// Plays a core working in another home than the user's (`mccboard --home`) when `true`:
    /// the Start menu is not offered, as the real environment does. `false` by default.
    pub fn set_other_home(&self, other: bool) {
        self.state().other_home = other;
    }

    /// Sets where mccboard's app is; `None`: it can't be found. [`FAKE_MCCBOARD_APP`] by
    /// default.
    pub fn set_mccboard_app(&self, program: Option<&Path>) {
        self.state().mccboard_app = program.map(Path::to_path_buf);
    }

    /// Makes writing a shortcut fail with `message`; `None` undoes it.
    pub fn set_shortcut_error(&self, message: Option<&str>) {
        self.state().shortcut_error = message.map(str::to_owned);
    }

    /// Every shortcut written so far, in order.
    pub fn shortcuts_written(&self) -> Vec<Shortcut> {
        self.state().shortcuts_written.clone()
    }

    /// Sets the launcher shim mccboard ships: other bytes, as after an upgrade of mccboard, or
    /// `None` when it isn't there.
    pub fn set_launcher_shim(&self, shim: Option<&[u8]>) {
        self.state().launcher_shim = shim.map(<[u8]>::to_vec);
    }

    /// Sets the CLI mccboard ships, a file the test wrote; `None` (the default): no CLI is
    /// shipped, so a sync on Windows leaves the bin folder's copy alone.
    pub fn set_shipped_cli(&self, cli: Option<&Path>) {
        self.state().shipped_cli = cli.map(Path::to_path_buf);
    }

    /// Turns Windows Developer Mode on or off: off, file links can't be created.
    pub fn set_developer_mode(&self, on: bool) {
        self.state().developer_mode = on;
    }

    /// Scripts what `claude` commands do: `handler` gets each invocation and returns its
    /// result, or an error as when `claude` can't be started. It may also write files, to
    /// play the part of Claude Code (a plugin install, a login). Without it, every command
    /// succeeds and prints nothing.
    pub fn on_claude(
        &self,
        handler: impl Fn(&ClaudeInvocation) -> io::Result<ClaudeRun> + Send + Sync + 'static,
    ) {
        self.state().claude_handler = ClaudeHandler(Arc::new(handler));
    }

    /// Every `claude` command run so far, in order.
    pub fn claude_invocations(&self) -> Vec<ClaudeInvocation> {
        self.state().claude_invocations.clone()
    }

    /// Scripts what the login does: `handler` gets each login and returns how it ended, or an
    /// error as when it can't be opened. It may write files, playing Claude Code. Without
    /// it, the user closes the login without logging in (exit code 1, nothing written).
    pub fn on_login(
        &self,
        handler: impl Fn(&LoginInvocation) -> io::Result<LoginExit> + Send + Sync + 'static,
    ) {
        self.state().login_handler = LoginHandler(Arc::new(handler));
    }

    /// Makes every login succeed as the account `uuid`, as Claude Code's does: it makes the
    /// instance's folder when it is missing (the primary instance's first login), adds
    /// `oauthAccount` to the instance's `.claude.json`, keeping every other key, and writes
    /// `.credentials.json` with [`FAKE_LOGIN_TOKEN`], valid for 8 hours from the fake clock.
    pub fn login_as(&self, uuid: &str, email: &str) {
        let (uuid, email) = (uuid.to_owned(), email.to_owned());
        // Weak, so the fake's state doesn't own itself through the handler.
        let state = Arc::downgrade(&self.state);
        self.on_login(move |login| {
            std::fs::create_dir_all(&login.config_dir)?;
            let global_config = match login.kind {
                InstanceKind::Linked => login.config_dir.join(".claude.json"),
                InstanceKind::Primary => login
                    .config_dir
                    .parent()
                    .unwrap_or(&login.config_dir)
                    .join(".claude.json"),
            };
            let mut config = std::fs::read_to_string(&global_config)
                .ok()
                .and_then(|text| serde_json::from_str::<Map<String, Value>>(&text).ok())
                .unwrap_or_default();
            config.insert(
                "oauthAccount".to_owned(),
                json!({
                    "accountUuid": uuid,
                    "emailAddress": email,
                    "organizationUuid": "00000000-0000-4000-8000-0000000000aa",
                }),
            );
            std::fs::write(&global_config, Value::Object(config).to_string())?;
            let now = state
                .upgrade()
                .map(|state| FakeEnvironment { state }.now())
                .unwrap_or_else(Timestamp::now);
            let expires_at = now + SignedDuration::from_hours(8);
            let credentials = json!({"claudeAiOauth": {
                "accessToken": FAKE_LOGIN_TOKEN,
                "refreshToken": FAKE_LOGIN_TOKEN,
                "expiresAt": expires_at.as_millisecond(),
                "scopes": ["user:inference", "user:profile"],
            }});
            std::fs::write(
                login.config_dir.join(".credentials.json"),
                credentials.to_string(),
            )?;
            Ok(LoginExit { exit_code: Some(0) })
        });
    }

    /// Every login opened so far, in order.
    pub fn login_invocations(&self) -> Vec<LoginInvocation> {
        self.state().login_invocations.clone()
    }

    /// Scripts what the token renewal does: `handler` gets each renewal and returns how it
    /// ended, or an error as when `claude` can't be started (`NotFound`) or hangs
    /// (`TimedOut`). It may rewrite `.credentials.json`, playing Claude Code renewing its
    /// token. Without it, Claude Code exits 0 and the token stays expired.
    pub fn on_renewal(
        &self,
        handler: impl Fn(&RenewalInvocation) -> io::Result<RenewalExit> + Send + Sync + 'static,
    ) {
        self.state().renewal_handler = RenewalHandler(Arc::new(handler));
    }

    /// Every token renewal run so far, in order.
    pub fn renewal_invocations(&self) -> Vec<RenewalInvocation> {
        self.state().renewal_invocations.clone()
    }

    /// Puts the program `name` (`claude`, `curl`, `bash`) on the fake PATH as `path`, or takes
    /// it off with `None`. The path need not exist: what the fake runs is scripted.
    pub fn set_program(&self, name: &str, path: Option<&Path>) {
        let mut state = self.state();
        match path {
            Some(path) => state.programs.insert(name.to_owned(), path.to_path_buf()),
            None => state.programs.remove(name),
        };
    }

    /// Scripts what `claude --version` does: `handler` gets the program and returns how the
    /// run ended, or an error as when it can't be started or hangs (`TimedOut`). Without it,
    /// it prints [`FAKE_CLAUDE_VERSION`] and exits 0.
    pub fn on_claude_version(
        &self,
        handler: impl Fn(&Path) -> io::Result<ClaudeRun> + Send + Sync + 'static,
    ) {
        self.state().version_handler = VersionHandler(Arc::new(handler));
    }

    /// The program of every `--version` run so far, in order.
    pub fn claude_version_runs(&self) -> Vec<PathBuf> {
        self.state().version_runs.clone()
    }

    /// Scripts what Claude Code's official installer does: `handler` gets each run and returns
    /// how it ended, or an error as when `curl` or `bash` can't be started. It may write files,
    /// playing the installer (`~/.local/bin/claude` in the invocation's home) or put `claude`
    /// on the PATH. Without it, the installer exits 0 and installs nothing.
    pub fn on_claude_install(
        &self,
        handler: impl Fn(&InstallerInvocation) -> io::Result<InstallerExit> + Send + Sync + 'static,
    ) {
        self.state().installer_handler = InstallerHandler(Arc::new(handler));
    }

    /// Every run of Claude Code's official installer so far, in order.
    pub fn installer_invocations(&self) -> Vec<InstallerInvocation> {
        self.state().installer_invocations.clone()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Environment for FakeEnvironment {
    fn now(&self) -> Timestamp {
        self.state().now
    }

    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply {
        let mut state = self.state();
        state.usage_calls.push(token.fingerprint());
        state
            .usage_replies
            .pop_front()
            .unwrap_or(EndpointReply::Unreachable {
                reason: "no scripted reply".to_owned(),
            })
    }

    fn os(&self) -> OsKind {
        self.state().os
    }

    fn login_shell(&self) -> Option<PathBuf> {
        self.state().login_shell.clone()
    }

    fn user_path_contains(&self, dir: &Path) -> io::Result<bool> {
        let state = self.state();
        if state.user_path_fails {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake: the user PATH can't be read",
            ));
        }
        Ok(state
            .user_path
            .iter()
            .any(|entry| crate::paths::same(entry, dir)))
    }

    fn add_to_user_path(&self, dir: &Path) -> io::Result<()> {
        let mut state = self.state();
        if state.user_path_fails {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake: the user PATH can't be changed",
            ));
        }
        state.user_path.push(dir.to_path_buf());
        state.user_path_additions += 1;
        Ok(())
    }

    fn launcher_shim(&self) -> io::Result<Vec<u8>> {
        self.state().launcher_shim.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "fake: mccboard-shim.exe isn't there",
            )
        })
    }

    fn shipped_cli(&self) -> io::Result<PathBuf> {
        self.state()
            .shipped_cli
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fake: no mccboard.exe"))
    }

    fn claude_processes(&self) -> Vec<ClaudeProcess> {
        self.state().processes.clone()
    }

    fn desktop_processes(&self) -> Vec<ProcessListing> {
        self.state().desktop_processes.clone()
    }

    fn desktop_app_packages(&self, families: &[String]) -> io::Result<Vec<DesktopPackage>> {
        let mut state = self.state();
        state.desktop_lookups.push(families.to_vec());
        match &state.desktop_lookup_error {
            Some(message) => Err(io::Error::other(format!("fake: {message}"))),
            None => Ok(state.desktop_packages.clone()),
        }
    }

    fn launch_desktop_app(&self, launch: &DesktopLaunch) -> io::Result<u32> {
        let (handler, count) = {
            let mut state = self.state();
            state.desktop_launches.push(launch.clone());
            (
                state.desktop_launch_handler.clone(),
                state.desktop_launches.len() as u32,
            )
        };
        match handler.0 {
            // Not under the lock: the handler may steer the fake itself.
            Some(handler) => handler(launch),
            None => Ok(FIRST_FAKE_DESKTOP_PID + count - 1),
        }
    }

    fn program_icon(&self, _program: &Path) -> io::Result<Image> {
        self.state()
            .program_icon
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fake: no program icon"))
    }

    fn start_menu_programs(&self, _home: &Path) -> io::Result<Option<PathBuf>> {
        let state = self.state();
        if state.other_home {
            return Ok(None);
        }
        state
            .start_menu
            .clone()
            .map(Some)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fake: no Start menu"))
    }

    fn mccboard_app(&self) -> io::Result<PathBuf> {
        self.state()
            .mccboard_app
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fake: no mccboard app"))
    }

    /// Records the shortcut and writes a placeholder file at its path (its parent folders
    /// created), so tests see it on disk as Windows would leave it.
    fn write_shortcut(&self, shortcut: &Shortcut) -> io::Result<()> {
        let mut state = self.state();
        if let Some(message) = &state.shortcut_error {
            return Err(io::Error::other(format!("fake: {message}")));
        }
        if let Some(parent) = shortcut.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&shortcut.path, format!("fake shell link: {shortcut:?}"))?;
        state.shortcuts_written.push(shortcut.clone());
        Ok(())
    }

    fn file_links_allowed(&self) -> bool {
        self.state().developer_mode
    }

    fn run_claude(&self, invocation: &ClaudeInvocation) -> io::Result<ClaudeRun> {
        let handler = {
            let mut state = self.state();
            state.claude_invocations.push(invocation.clone());
            state.claude_handler.clone()
        };
        // Not under the lock: the handler may steer the fake itself.
        (handler.0)(invocation)
    }

    fn open_login(&self, login: &LoginInvocation) -> io::Result<LoginExit> {
        let handler = {
            let mut state = self.state();
            state.login_invocations.push(login.clone());
            state.login_handler.clone()
        };
        (handler.0)(login)
    }

    fn renew_token(&self, renewal: &RenewalInvocation) -> io::Result<RenewalExit> {
        let handler = {
            let mut state = self.state();
            state.renewal_invocations.push(renewal.clone());
            state.renewal_handler.clone()
        };
        (handler.0)(renewal)
    }

    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.state().programs.get(name).cloned()
    }

    fn claude_version(&self, program: &Path) -> io::Result<ClaudeRun> {
        let handler = {
            let mut state = self.state();
            state.version_runs.push(program.to_path_buf());
            state.version_handler.clone()
        };
        (handler.0)(program)
    }

    fn install_claude_code(&self, install: &InstallerInvocation) -> io::Result<InstallerExit> {
        let handler = {
            let mut state = self.state();
            state.installer_invocations.push(install.clone());
            state.installer_handler.clone()
        };
        (handler.0)(install)
    }
}
