//! Test support: the fake environment port. Enabled by the `testing` feature.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::{SignedDuration, Timestamp};

use serde_json::{Map, Value, json};

use crate::{
    AccessToken, ClaudeInvocation, ClaudeProcess, ClaudeRun, DesktopLaunch, DesktopPackage,
    EndpointReply, Environment, Image, InstanceKind, LoginExit, LoginInvocation, OsKind,
    ProcessListing, Shortcut,
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

/// A scriptable [`Environment`] for tests. Clones share state, so a test can keep a handle
/// after giving one to [`crate::Core::new`] and keep steering it.
///
/// It starts as a Linux machine with no login shell and an empty user PATH, whatever OS the
/// tests run on, shipping [`FAKE_LAUNCHER_SHIM`] as the launcher shim.
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
    mccboard_app: Option<PathBuf>,
    shortcuts_written: Vec<Shortcut>,
    shortcut_error: Option<String>,
    developer_mode: bool,
    claude_invocations: Vec<ClaudeInvocation>,
    claude_handler: ClaudeHandler,
    login_invocations: Vec<LoginInvocation>,
    login_handler: LoginHandler,
}

type HandlerFn = dyn Fn(&ClaudeInvocation) -> io::Result<ClaudeRun> + Send + Sync;

/// What a fake `claude` command does; by default it succeeds and prints nothing.
#[derive(Clone)]
struct ClaudeHandler(Arc<HandlerFn>);

type LoginFn = dyn Fn(&LoginInvocation) -> io::Result<LoginExit> + Send + Sync;

/// What a fake login does; by default the user closes it without logging in.
#[derive(Clone)]
struct LoginHandler(Arc<LoginFn>);

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
                processes: Vec::new(),
                desktop_processes: Vec::new(),
                desktop_packages: Vec::new(),
                desktop_lookup_error: None,
                desktop_lookups: Vec::new(),
                desktop_launches: Vec::new(),
                desktop_launch_handler: DesktopLaunchHandler(None),
                program_icon: None,
                start_menu: None,
                mccboard_app: Some(PathBuf::from(FAKE_MCCBOARD_APP)),
                shortcuts_written: Vec::new(),
                shortcut_error: None,
                developer_mode: true,
                claude_invocations: Vec::new(),
                claude_handler: ClaudeHandler(Arc::new(|_| Ok(ClaudeRun::ok("")))),
                login_invocations: Vec::new(),
                login_handler: LoginHandler(Arc::new(|_| Ok(LoginExit { exit_code: Some(1) }))),
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

    /// Makes every login succeed as the account `uuid`, as Claude Code's does: it adds
    /// `oauthAccount` to the instance's `.claude.json`, keeping every other key, and writes
    /// `.credentials.json` with [`FAKE_LOGIN_TOKEN`], valid for 8 hours from the fake clock.
    pub fn login_as(&self, uuid: &str, email: &str) {
        let (uuid, email) = (uuid.to_owned(), email.to_owned());
        // Weak, so the fake's state doesn't own itself through the handler.
        let state = Arc::downgrade(&self.state);
        self.on_login(move |login| {
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

    fn start_menu_programs(&self) -> io::Result<PathBuf> {
        self.state()
            .start_menu
            .clone()
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
}
