//! mccboard's core: domain, data sources, setup operations and token rules.
//!
//! The CLI and the Tauri app are thin front ends over [`Core`]; all behavior lives here.
//! The core never depends on Tauri (ADR 0004). It reaches everything outside the filesystem
//! through one port, [`Environment`], and takes the home directory as a parameter, so tests
//! run against temporary homes with [`testing::FakeEnvironment`].
//!
//! Token rules (AGENTS.md, ADR 0002): credentials are read-only, and no token ever appears in
//! anything this crate returns, prints or persists.

mod account;
mod add;
mod alignment;
mod atomic_file;
mod background;
mod backup;
mod claude_cli;
mod claude_files;
mod config;
mod desktop_app;
mod desktop_identity;
mod desktop_launch;
mod desktop_process;
mod desktop_profile;
mod desktop_tray;
mod desktop_watch;
mod doctor;
mod environment;
mod error;
mod folder_guard;
mod icon;
mod instance;
mod language;
mod launcher;
mod links;
mod paths;
mod registry;
mod remove;
mod repair;
mod scan;
mod session_guard;
mod sharing;
mod system;
mod taskbar_widget;
#[cfg(feature = "testing")]
pub mod testing;
mod tray;
mod usage;
#[cfg(windows)]
mod windows_desktop_app;
#[cfg(windows)]
pub mod windows_profile_tray;
#[cfg(windows)]
mod windows_shell;
#[cfg(windows)]
pub mod windows_taskbar;
#[cfg(windows)]
pub mod windows_taskbar_widget;
#[cfg(windows)]
mod windows_user_path;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use account::{
    Account, AccountUpdate, AccountUuid, CLAUDE_ORANGE, Color, NEUTRAL_COLOR, ObservedAccount,
};
pub use add::{AddReport, AddRequest, LoginReport, LoginRun, LoginWarning};
pub use alignment::GLOBAL_CONFIG_KEYS;
pub use desktop_identity::{
    AppArgs, OPEN_PROFILE_ARG, RunningProfile, Saved, Shortcut, ShortcutSync, ShortcutWarning,
    ShortcutsNotSynced, WindowIdentity, app_user_model_id, command_line, open_profile_args,
    shortcut_file_name,
};
pub use desktop_launch::{DesktopLaunch, DesktopPackage, PACKAGE_FAMILY};
pub use desktop_process::{DesktopDataDir, DesktopProcess, ProcessListing};
pub use desktop_profile::{
    DesktopObservation, DesktopOpening, DesktopProfile, DesktopProfileRemoval,
    DesktopProfileUpdate, DesktopStartOpening, DesktopWarning,
};
pub use desktop_tray::{
    IconAddress, PROFILE_TRAY_AFTER_TASKBAR_CREATED, PROFILE_TRAY_REAPPLY_EVERY, ProfileTrayIcon,
    ProfileTrayKeeper, ProfileTrayRepaint, RepaintOutcome, profile_tray_icons,
    profile_tray_tooltip,
};
pub use desktop_watch::{DesktopNotice, DesktopNoticeKind, DesktopNotices};
pub use doctor::{Doctor, DriftItem, InstanceDrift, Problem, Repair};
pub use environment::{
    ClaudeInvocation, ClaudeRun, EndpointReply, Environment, LoginExit, LoginInvocation,
    LoginTerminal, OsKind, RenewalExit, RenewalInvocation, SystemEnvironment,
};
pub use error::Error;
pub use icon::{
    IconCache, IconSources, Image, LogoStyle, PackageLogos, ProfileIcons, mccboard_logo,
};
pub use instance::{InstanceKind, LoginState, Observation, Warning};
pub use language::Language;
pub use launcher::{
    CliInstall, InstanceLauncher, Launcher, LauncherPlace, LauncherState, LauncherStatus,
    LauncherSync, UserPathChange,
};
pub use paths::Paths;
pub use registry::{AccountStatus, InstanceStatus, Registration, Registry};
pub use remove::{FolderOutcome, Removal, RemoveLevel};
pub use repair::{RepairOutcome, RepairReport, RepairedItem};
pub use scan::Candidate;
pub use session_guard::{ClaudeProcess, ProcessConfigDir};
pub use sharing::{SharingAnswer, SharingQuestion};
pub use taskbar_widget::{
    AccountInk, ColumnState, FADED_OPACITY, ResetPart, SEGMENTS, TaskbarTheme, TaskbarWidget,
    TimeToReset, WidgetColumn, WidgetPalette, WidgetRow,
};
pub use tray::profiles::{TrayProfile, TrayProfiles};
pub use tray::{
    LanguageChoice, TOOLTIP_MAX_UTF16, TrayAccount, TrayIcon, TrayMenu, TrayView, TrayWindow,
    WindowName,
};
pub use usage::{
    AccessToken, AccountUsage, Freshness, ModelWindow, RenewalAttempt, RenewalFailure,
    RenewalOutcome, RenewalSituation, UsageBlocker, UsageFailure, UsageReading, UsageReport,
    UsageSituation, UsageSource, UsageWarning, UsageWindow,
};

/// The entry point to every core operation, bound to one home directory on the local machine.
#[derive(Clone)]
pub struct Core {
    home: PathBuf,
    env: Arc<dyn Environment>,
    /// Serializes usage refreshes within this process (rate limits are per account).
    usage_lock: Arc<Mutex<()>>,
    /// Serializes shortcut syncs within this process (they share a record file).
    shortcut_lock: Arc<Mutex<()>>,
    /// What the desktop profile watch remembers between polls ([`Core::poll_desktop_profiles`]).
    desktop_watch: Arc<Mutex<desktop_watch::Watch>>,
}

impl Core {
    /// A core working in `home` (the user's home directory, or a temporary one in tests).
    pub fn new(home: impl Into<PathBuf>, env: impl Environment + 'static) -> Self {
        Self {
            home: home.into(),
            env: Arc::new(env),
            usage_lock: Arc::default(),
            shortcut_lock: Arc::default(),
            desktop_watch: Arc::default(),
        }
    }

    /// The home directory this core works in.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where mccboard keeps its own files in this home.
    pub fn paths(&self) -> Paths {
        Paths::new(&self.home)
    }

    /// Finds instance candidates in the home: `~/.claude` (the primary instance) and every
    /// `~/.claude-*` directory (linked instances), primary first. Scanning only proposes
    /// instances: it registers nothing and writes nothing. Each candidate says whether it is
    /// registered already.
    pub fn scan(&self) -> Result<Vec<Candidate>, Error> {
        let registered = registry::registered_dirs(&self.paths().config_file())?;
        scan::scan(&self.home, self.env.as_ref(), &registered)
    }

    /// The registered accounts and instances, each instance observed now, with the token
    /// renewal that runs in it. Reads `config.toml` and mccboard's state, and writes nothing.
    pub fn registry(&self) -> Result<Registry, Error> {
        let mut registry =
            registry::registry(&self.home, &self.paths().config_file(), self.env.as_ref())?;
        // Without the usage state (unreadable), the instances just show no renewal.
        if let Ok(report) = self.usage() {
            for instance in &mut registry.instances {
                instance.renewal = report.renewal_in(&instance.config_dir);
            }
        }
        Ok(registry)
    }

    /// Registers the instance whose config directory is `path` (absolute, or starting with
    /// `~`), at any location, logged in or not. The instance is recorded with the account it
    /// is logged in with; an account seen for the first time is added with a suggested label
    /// and color. Registering a registered instance again is harmless and accepts the account
    /// it is now logged in with.
    ///
    /// A linked instance gets its launcher (see [`Core::sync_launchers`]). If the launchers
    /// can't be written, the error says so; the registration is saved all the same.
    ///
    /// An instance added without its login keeps the account label asked for in `config.toml`
    /// until its account is first observed: registering it then gives that label to the
    /// account, if the account is new and no other account has the label.
    pub fn register(&self, path: &Path) -> Result<Registration, Error> {
        self.register_labelled(path, None)
    }

    /// [`Core::register`], with `label` waiting for the instance's account (see
    /// [`Core::add`]).
    fn register_labelled(&self, path: &Path, label: Option<&str>) -> Result<Registration, Error> {
        let registration = registry::register(
            &self.home,
            &self.paths().config_file(),
            self.env.as_ref(),
            path,
            label,
        )?;
        self.sync_launchers()?;
        Ok(registration)
    }

    /// Forgets a registered instance and removes its launcher. Nothing else on disk changes
    /// but `config.toml`; the account keeps its label and color. The same as
    /// [`Core::remove`] with [`RemoveLevel::FromMccboard`]: the primary instance is refused.
    pub fn unregister(&self, path: &Path) -> Result<(), Error> {
        self.remove(path, RemoveLevel::FromMccboard).map(drop)
    }

    /// Removes the registered linked instance in `path` (absolute, or starting with `~`):
    /// - [`RemoveLevel::FromMccboard`] forgets it and removes its launcher; its folder stays
    ///   as it is;
    /// - [`RemoveLevel::WithFolder`] also deletes its folder, and so the account's login on
    ///   this machine. Front ends confirm this with the user first. It refuses while a Claude
    ///   Code process uses the folder ([`Error::InstanceInUse`]), and for a folder that holds
    ///   the home, the primary instance or mccboard's files, or is a link
    ///   ([`Error::FolderNotDeletable`]). Links inside are removed, never followed, so the
    ///   primary instance's items stay intact.
    ///
    /// The primary instance is never removed ([`Error::PrimaryInstance`]). The account keeps
    /// its label and color. If the launchers can't be written, the error says so; the
    /// instance is removed all the same.
    pub fn remove(&self, path: &Path, level: RemoveLevel) -> Result<Removal, Error> {
        remove::remove(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            path,
            level,
            &|| self.sync_launchers(),
        )
    }

    /// Changes an account's label, color or launcher name override. The launchers of its
    /// linked instances follow the new name, and its desktop profile's Start-menu shortcut
    /// and icon the new label and color ([`Core::sync_desktop_shortcuts`]). The default
    /// desktop profile's account (the primary instance's) always shows Claude's orange: a new
    /// color for it is refused ([`Error::DefaultProfileColor`]). If the launchers can't be
    /// written, the error says so; the account is changed all the same. A shortcut that
    /// can't follow is only a warning in the result ([`Saved`]).
    pub fn update_account(
        &self,
        uuid: &AccountUuid,
        update: AccountUpdate,
    ) -> Result<Saved<Account>, Error> {
        let account =
            registry::update_account(&self.home, &self.paths().config_file(), uuid, update)?;
        let launchers = self.sync_launchers();
        let saved = Saved::new(account, self.sync_desktop_shortcuts());
        launchers?;
        Ok(saved)
    }

    /// Every desktop profile, the default one first, then the others by label: each with its
    /// account, color, data folder, instance, observed account and warnings. The default
    /// desktop profile is implicit (the app's own data directory, the primary instance) and
    /// shows Claude's orange. Reads `config.toml` and lists the `claude-code-sessions`
    /// folders of the data folders; opens no file in them and writes nothing.
    pub fn desktop_profiles(&self) -> Result<Vec<DesktopProfile>, Error> {
        desktop_profile::list(&self.home, &self.paths().config_file(), self.env.as_ref())
    }

    /// The registered accounts that can get a desktop profile, in label order: those with a
    /// local instance and no profile yet, but the default desktop profile's. For front ends
    /// to offer when adding one.
    pub fn desktop_profile_candidates(&self) -> Result<Vec<Account>, Error> {
        desktop_profile::candidates(&self.home, &self.paths().config_file())
    }

    /// Opens every desktop profile set to open when mccboard starts and not running already,
    /// for the app to call once at its start. One profile failing doesn't stop the others:
    /// each opening's result is in the list. Off Windows, opens nothing.
    pub fn open_desktop_profiles_at_start(&self) -> Result<Vec<DesktopStartOpening>, Error> {
        desktop_profile::open_at_start(&self.home, &self.paths(), self.env.as_ref())
    }

    /// The data folder a new desktop profile for `account` gets unless told otherwise:
    /// `%LOCALAPPDATA%\Claude-<label>` on Windows (`~/.config/Claude-<label>` elsewhere, where
    /// it is never used). For front ends to show before adding.
    pub fn desktop_profile_folder(&self, account: &AccountUuid) -> Result<PathBuf, Error> {
        desktop_profile::default_folder(
            &self.home,
            &self.paths().config_file(),
            self.env.as_ref(),
            account,
        )
    }

    /// Adds a desktop profile for the registered `account`, with its data folder in
    /// `data_folder` (absolute, or starting with `~`), or in the default one
    /// ([`Core::desktop_profile_folder`]). Its Code tab will use the account's local
    /// instance. Only `config.toml` changes: the folder is created when the profile first
    /// opens. Refuses, before changing anything, an account with no local instance
    /// ([`Error::NoLocalInstance`]), one that has a profile already
    /// ([`Error::DesktopProfileExists`]) or is the primary instance's, whose profile is the
    /// default one ([`Error::DefaultDesktopProfileAccount`]), and a data folder inside or
    /// holding an instance, mccboard's files, the app's own data or another profile's folder,
    /// or that is a link ([`Error::DesktopProfileFolderNotAllowed`]).
    ///
    /// On Windows the profile gets its Start-menu shortcut ([`Core::sync_desktop_shortcuts`]);
    /// if it can't be written, the result carries why ([`Saved`]) and the profile is added
    /// all the same.
    pub fn add_desktop_profile(
        &self,
        account: &AccountUuid,
        data_folder: Option<&Path>,
    ) -> Result<Saved<DesktopProfile>, Error> {
        let profile = desktop_profile::add(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            account,
            data_folder,
        )?;
        Ok(Saved::new(profile, self.sync_desktop_shortcuts()))
    }

    /// Changes `account`'s desktop profile (whether it opens when mccboard starts), keeping
    /// everything else in `config.toml`. The default desktop profile is never changed
    /// ([`Error::DefaultDesktopProfile`]).
    pub fn update_desktop_profile(
        &self,
        account: &AccountUuid,
        update: DesktopProfileUpdate,
    ) -> Result<DesktopProfile, Error> {
        desktop_profile::update(
            &self.home,
            &self.paths().config_file(),
            self.env.as_ref(),
            account,
            update,
        )
    }

    /// Removes `account`'s desktop profile:
    /// - [`RemoveLevel::FromMccboard`] forgets it; its data folder stays as it is;
    /// - [`RemoveLevel::WithFolder`] also deletes its data folder, and with it the profile's
    ///   sign-in on this machine. Front ends confirm this with the user first. It refuses
    ///   while a process of the desktop app uses the folder ([`Error::DesktopProfileInUse`]),
    ///   and for a folder that the rules of [`Core::add_desktop_profile`] wouldn't accept
    ///   ([`Error::FolderNotDeletable`]). Links inside are removed, never followed.
    ///
    /// Its Start-menu shortcut goes with it ([`Core::sync_desktop_shortcuts`]; if that
    /// fails, the result carries why ([`Saved`]) and the profile is removed all the same),
    /// then its cached icons ([`IconCache::remove`]); one that can't be deleted stays behind
    /// without failing the removal.
    ///
    /// The default desktop profile is never removed ([`Error::DefaultDesktopProfile`]).
    pub fn remove_desktop_profile(
        &self,
        account: &AccountUuid,
        level: RemoveLevel,
    ) -> Result<Saved<DesktopProfileRemoval>, Error> {
        let paths = self.paths();
        let removal =
            desktop_profile::remove(&self.home, &paths, self.env.as_ref(), account, level)?;
        // The shortcut first: it shows the icon.
        let saved = Saved::new(removal, self.sync_desktop_shortcuts());
        // Leftover icons are harmless (a new color or profile gets new file names).
        let _ = IconCache::new(&paths).remove(account);
        Ok(saved)
    }

    /// Makes the desktop profiles' Start-menu shortcuts match the stored profiles, on Windows
    /// (elsewhere it does nothing): `Claude <label>.lnk` in the Start menu's Programs folder,
    /// starting mccboard's app with `--open-profile <uuid>` ([`AppArgs`]), with the profile's
    /// AppUserModelID ([`app_user_model_id`]) and colored icon, drawn and cached as needed. A
    /// shortcut whose label, color or target changed is rewritten (renamed: the new one
    /// written, the old one deleted); the shortcuts mccboard made that no profile needs are
    /// deleted; files it didn't make are never touched. Pinning stays the user's.
    ///
    /// Adding, changing and removing profiles and accounts call it; the app also calls it when
    /// it starts and when `config.toml` changes. Running it again changes nothing. A core
    /// working in another home than the user's (`mccboard --home`) makes no shortcut. Fails when
    /// the Start menu ([`Error::StartMenuNotFound`]) or mccboard's app
    /// ([`Error::AppProgramNotFound`]) can't be found, or a shortcut can't be written
    /// ([`Error::ShortcutNotWritten`]); an icon that can't be drawn is only a warning.
    pub fn sync_desktop_shortcuts(&self) -> Result<ShortcutSync, Error> {
        let _guard = self
            .shortcut_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        desktop_identity::sync(&self.home, &self.paths(), self.env.as_ref())
    }

    /// The desktop profiles running now, the default one never among them, each with every
    /// process of the app running it and what their windows are tagged with: the profile's
    /// AppUserModelID, `Claude <label>`, mccboard's relaunch command and its colored icon,
    /// drawn and cached as needed. For the app's taskbar watcher (Windows), which calls it
    /// whenever processes may have changed. Lists the desktop app's processes.
    pub fn running_desktop_profiles(&self) -> Result<Vec<RunningProfile>, Error> {
        desktop_identity::running(&self.home, &self.paths(), self.env.as_ref())
    }

    /// Opens `account`'s desktop profile on Windows, or the default desktop profile when
    /// `account` is the primary instance's. The app is found from its MSIX package at every
    /// launch, since its folder changes with each update. The data folder is created first,
    /// then the app starts with `--user-data-dir=<folder>` and, in its environment,
    /// `CLAUDE_CONFIG_DIR=<the account's local instance>`, every other inherited `CLAUDE*`
    /// and `ANTHROPIC*` variable removed, with no console window. mccboard doesn't wait for
    /// it: a profile open already comes to the front. The report carries the pid started.
    ///
    /// Refuses off Windows ([`Error::DesktopAppNeedsWindows`]), when the app isn't installed
    /// ([`Error::DesktopAppNotInstalled`]) or only the old installer's is
    /// ([`Error::DesktopAppNotMsix`]), for an account without a profile
    /// ([`Error::DesktopProfileNotFound`]) or without a local instance
    /// ([`Error::NoLocalInstance`]), and for a hand-edited data folder the rules of
    /// [`Core::add_desktop_profile`] wouldn't accept.
    pub fn open_desktop_profile(&self, account: &AccountUuid) -> Result<DesktopOpening, Error> {
        desktop_profile::open(&self.home, &self.paths(), self.env.as_ref(), Some(account))
    }

    /// Opens the default desktop profile: the app as it starts normally, with neither
    /// `--user-data-dir` nor `CLAUDE_CONFIG_DIR` (the inherited `CLAUDE*`/`ANTHROPIC*`
    /// variables are still removed). Refuses as [`Core::open_desktop_profile`] does.
    pub fn open_default_desktop_profile(&self) -> Result<DesktopOpening, Error> {
        desktop_profile::open(&self.home, &self.paths(), self.env.as_ref(), None)
    }

    /// Looks at the desktop app's processes once, compares them with the previous look and
    /// with the processes mccboard launched (remembered in `~/.mccboard/state/`), and returns
    /// the notices showing, in `language`:
    /// - a stored profile running from a process mccboard didn't start, probably the app's
    ///   restart after an update ([`DesktopNoticeKind::ReopenedWithoutMccboard`]): its Code tab
    ///   may lack its instance;
    /// - a profile that vanished while, within a minute, a new copy of the default profile
    ///   appeared ([`DesktopNoticeKind::ClosedByUpdate`]).
    ///
    /// Front ends call it every few seconds; one look alone can only tell the first case. A
    /// notice goes once the profile runs again from a launch by mccboard, or when dismissed
    /// ([`Core::dismiss_desktop_notice`]). Nothing is ever closed. Without stored profiles,
    /// no process is listed.
    pub fn poll_desktop_profiles(&self, language: Language) -> Result<DesktopNotices, Error> {
        desktop_watch::poll(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            &self.desktop_watch,
            language,
        )
    }

    /// The desktop profile notices as of the last [`Core::poll_desktop_profiles`], in
    /// `language`. Lists no process.
    pub fn desktop_notices(&self, language: Language) -> Result<DesktopNotices, Error> {
        desktop_watch::notices(&self.paths(), &self.desktop_watch, language)
    }

    /// Hides the notice of `account`'s desktop profile until it happens again; `false` when
    /// none was showing.
    pub fn dismiss_desktop_notice(&self, account: &AccountUuid) -> bool {
        desktop_watch::dismiss(&self.desktop_watch, account)
    }

    /// The processes of the Claude desktop app using the data folder `data_folder`
    /// (absolute, or starting with `~`): a desktop profile running. While there are any, its
    /// folder isn't deleted.
    pub fn desktop_processes_using(
        &self,
        data_folder: &Path,
    ) -> Result<Vec<DesktopProcess>, Error> {
        let data_folder = paths::resolve(&self.home, data_folder)?;
        Ok(desktop_profile::processes_using(
            &self.home,
            self.env.as_ref(),
            &data_folder,
        ))
    }

    /// The launchers as they are, against what the registered linked instances need: one per
    /// instance (ok, missing or stale), plus launchers mccboard made that nothing needs any
    /// more. Reads only. The doctor reports a missing or stale launcher as drift.
    pub fn launchers(&self) -> Result<LauncherStatus, Error> {
        launcher::status(&self.home, &self.paths(), self.env.as_ref())
    }

    /// Makes the launchers match the registered linked instances: creates the missing ones,
    /// rewrites stale ones, removes the ones nothing needs, and on Windows puts the bin folder
    /// on the user PATH once. Touches only mccboard's block of the shell rc file (Linux) or
    /// mccboard's bin folder (Windows). Running it again changes nothing.
    pub fn sync_launchers(&self) -> Result<LauncherSync, Error> {
        launcher::sync(&self.home, &self.paths(), self.env.as_ref())
    }

    /// On Windows, puts mccboard's CLI on the PATH: copies the CLI shipped with mccboard
    /// (`mccboard.exe`, next to the app) into the bin folder when it is missing there or
    /// differs, moving a running copy aside, and deletes what earlier syncs moved aside. The
    /// app calls it when it starts, so an update of the app refreshes the CLI; every
    /// [`Core::sync_launchers`] does it too. Never fails: what went wrong is in the result.
    /// `None` on Linux, where the CLI isn't copied anywhere.
    pub fn install_cli(&self) -> Option<CliInstall> {
        launcher::install_cli(&self.paths(), self.env.as_ref())
    }

    /// Observes the account and login state of the instance whose config directory is
    /// `config_dir`. Missing or unreadable files never fail: they show in the observation.
    pub fn observe(&self, config_dir: &Path) -> Observation {
        instance::observe(&self.home, config_dir, self.env.as_ref())
    }

    /// Whether `config_dir` is the primary instance's (Claude Code's default) or a linked one.
    /// A link to the primary's folder, or a path that leads there through links, is the
    /// primary instance too: it is never linked or repaired as a linked instance.
    pub fn kind_of(&self, config_dir: &Path) -> InstanceKind {
        InstanceKind::of(&self.home, config_dir)
    }

    /// The drift of every registered instance against the sharing table (ADR 0005), each
    /// item explained with the repair that would fix it, plus the unknown items of the
    /// primary instance waiting for an answer. Reads only; changes nothing.
    ///
    /// A registered linked instance whose launcher is missing or stale has a `launcher:<name>`
    /// item, repaired by [`Core::sync_launchers`]. If the launchers can't be read, the report
    /// leaves them out and [`Core::launchers`] says why.
    pub fn doctor(&self) -> Result<Doctor, Error> {
        let launchers = self.launchers().ok();
        doctor::doctor(&self.home, &self.paths().config_file(), launchers.as_ref())
    }

    /// The drift of the instance in `config_dir` (absolute, or starting with `~`), registered
    /// or not. Reads only; changes nothing.
    pub fn drift(&self, config_dir: &Path) -> Result<InstanceDrift, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        let config = config::ConfigFile::load(&self.paths().config_file())?.config;
        let launchers = self.launchers().ok();
        Ok(doctor::drift(
            &self.home,
            &config_dir,
            &config,
            launchers.as_ref(),
        ))
    }

    /// Unknown top-level items of the primary instance the user hasn't answered about yet:
    /// "share `gsd-core`?". Each is asked once: see [`Core::answer_sharing`].
    pub fn sharing_questions(&self) -> Result<Vec<SharingQuestion>, Error> {
        let answers = config::ConfigFile::load(&self.paths().config_file())?
            .config
            .sharing;
        Ok(sharing::questions(
            &doctor::primary_dir(&self.home),
            &answers,
        ))
    }

    /// Remembers in `config.toml` whether the unknown item `item` (a name in the primary
    /// instance's folder) is linked or kept private, for every instance. Answering again
    /// changes the answer. Items in the sharing table take no answer.
    pub fn answer_sharing(&self, item: &str, answer: SharingAnswer) -> Result<(), Error> {
        sharing::validate_item(item)?;
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_sharing(item, answer);
        file.save()
    }

    /// Every registered account's last known usage reading, without calling the endpoint.
    /// Windows whose reset time has passed read as zero. It doesn't wait for a refresh in
    /// progress: it shows what was saved last.
    pub fn usage(&self) -> Result<UsageReport, Error> {
        usage::report(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            &self.usage_lock,
            false,
        )
    }

    /// Refreshes usage, then reports it like [`Core::usage`]. The endpoint is called once per
    /// account, with a token from one of its registered instances, and only where the rate
    /// limits allow it: a manual refresh follows the same rules as a periodic one. A failed
    /// call keeps the last reading, marked stale.
    ///
    /// When every token of an account has expired and the preference allows it
    /// ([`Core::renew_expired_tokens`]), Claude Code is first asked to renew the token in the
    /// account's usage instance (`claude mcp get` with a missing name, hidden), at most once per instance
    /// every 15 minutes, backing off up to 6 hours after failures. The token is then read
    /// again, and the endpoint called with it if it was renewed. mccboard never refreshes a
    /// token or writes credentials itself.
    pub fn refresh_usage(&self) -> Result<UsageReport, Error> {
        usage::report(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            &self.usage_lock,
            true,
        )
    }

    /// The Claude Code processes using the instance in `config_dir` (absolute, or starting
    /// with `~`). While there are any, operations that change the instance refuse.
    pub fn processes_using(&self, config_dir: &Path) -> Result<Vec<ClaudeProcess>, Error> {
        let config_dir = paths::resolve(&self.home, config_dir)?;
        Ok(session_guard::processes_using(
            &self.home,
            self.env.as_ref(),
            &config_dir,
        ))
    }

    /// Repairs the drift of the registered instance in `config_dir`: every item, or only the
    /// one whose id is `item` ([`DriftItem::id`]). Refuses before changing anything while a
    /// Claude Code process uses the instance ([`Error::InstanceInUse`]), when a file link is
    /// needed and Windows Developer Mode is off ([`Error::DeveloperModeOff`]), or when a link
    /// would land in the Claude desktop app's data folder. Real files and folders a link
    /// replaces are moved to a backup under `~/.mccboard/state/backups/` first. Repairing
    /// again changes nothing; the primary instance is never relinked.
    ///
    /// Aligned items are brought in line with the primary instance: a Global config key
    /// (`config:<key>`) by writing only that key into the instance's `.claude.json`, a
    /// marketplace or plugin (`marketplace:<name>`, `plugin:<id>`) by running Claude Code's
    /// own `claude plugin` commands in the instance. A failed install is reported in its item
    /// and doesn't stop the others.
    ///
    /// A missing or stale launcher (`launcher:<name>`) is repaired with
    /// [`Core::sync_launchers`]; launchers live outside the instance's folder, so repairing
    /// only them doesn't wait for its sessions to end.
    pub fn repair(&self, config_dir: &Path, item: Option<&str>) -> Result<RepairReport, Error> {
        repair::repair(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            config_dir,
            item,
            &|| self.launchers().ok(),
            &|| self.sync_launchers(),
        )
    }
    /// The app's language as chosen in `config.toml`; `None` when the user hasn't chosen one
    /// (front ends then follow the system: see [`Language::from_locale`]).
    pub fn language(&self) -> Result<Option<Language>, Error> {
        Ok(config::ConfigFile::load(&self.paths().config_file())?
            .config
            .language)
    }

    /// Saves the app's language in `config.toml`, keeping everything else in the file.
    pub fn set_language(&self, language: Language) -> Result<(), Error> {
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_language(language);
        file.save()
    }

    /// Whether a usage refresh may ask Claude Code to renew an expired token
    /// (`renew_expired_tokens` in `config.toml`'s `[preferences]`; on unless set to false).
    pub fn renew_expired_tokens(&self) -> Result<bool, Error> {
        Ok(config::ConfigFile::load(&self.paths().config_file())?
            .config
            .renew_expired_tokens)
    }

    /// Saves whether a usage refresh may ask Claude Code to renew an expired token, keeping
    /// everything else in `config.toml`.
    pub fn set_renew_expired_tokens(&self, on: bool) -> Result<(), Error> {
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_renew_expired_tokens(on);
        file.save()
    }

    /// Whether the taskbar widget is shown (`taskbar_widget` in `config.toml`'s
    /// `[preferences]`; on unless set to false). Windows only: elsewhere it has no effect.
    pub fn taskbar_widget_on(&self) -> Result<bool, Error> {
        Ok(config::ConfigFile::load(&self.paths().config_file())?
            .config
            .taskbar_widget)
    }

    /// Saves whether the taskbar widget is shown, keeping everything else in `config.toml`.
    pub fn set_taskbar_widget_on(&self, on: bool) -> Result<(), Error> {
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_taskbar_widget(on);
        file.save()
    }

    /// Whether the taskbar widget shows each account's label (`taskbar_widget_labels` in
    /// `config.toml`'s `[preferences]`; off unless set to true).
    pub fn taskbar_widget_labels(&self) -> Result<bool, Error> {
        Ok(config::ConfigFile::load(&self.paths().config_file())?
            .config
            .taskbar_widget_labels)
    }

    /// Saves whether the taskbar widget shows each account's label, keeping everything else
    /// in `config.toml`.
    pub fn set_taskbar_widget_labels(&self, on: bool) -> Result<(), Error> {
        let mut file = config::ConfigFile::load(&self.paths().config_file())?;
        file.set_taskbar_widget_labels(on);
        file.save()
    }

    /// What the taskbar widget shows for `report` (from [`Core::usage`] or
    /// [`Core::refresh_usage`]), as of now: one column per account in `config.toml` order,
    /// each with its session and weekly windows' bar, pace and time to reset, its state and
    /// its colors for both taskbar themes; the tooltip in `language` with reset times shown in
    /// `tz`; and when to compute it again. Reads `config.toml`; calls nothing.
    pub fn taskbar_widget(
        &self,
        report: &UsageReport,
        language: Language,
        tz: &jiff::tz::TimeZone,
    ) -> Result<TaskbarWidget, Error> {
        let config = config::ConfigFile::load(&self.paths().config_file())?.config;
        Ok(taskbar_widget::view(
            report,
            &config.account_order,
            config.taskbar_widget,
            config.taskbar_widget_labels,
            language,
            tz,
            self.env.now(),
        ))
    }

    /// What the tray shows for `report` (from [`Core::usage`] or [`Core::refresh_usage`]),
    /// as of now: the weekly window and color of the account closest to its weekly limit for
    /// the icon, a tooltip that fits the Windows tray, every account's windows as menu lines,
    /// and "no instances" when none is registered. Texts are in `language`, with reset times shown in `tz`. Windows whose
    /// reset passed since the report read as zero. Reads `config.toml`; calls nothing.
    pub fn tray_view(
        &self,
        report: &UsageReport,
        language: Language,
        tz: &jiff::tz::TimeZone,
    ) -> Result<TrayView, Error> {
        let config = config::ConfigFile::load(&self.paths().config_file())?.config;
        Ok(tray::view(
            report,
            config.instances.is_empty(),
            config.renew_expired_tokens,
            language,
            tz,
            self.env.now(),
        ))
    }

    /// The tray's "Open profile" submenu in `language`: every desktop profile, the default one
    /// first, with its account's color and whether it still needs its first sign-in. Empty
    /// off Windows, where the app doesn't run. Lists the desktop app's processes.
    pub fn tray_profiles(&self, language: Language) -> Result<TrayProfiles, Error> {
        if self.env.os() != OsKind::Windows {
            return Ok(tray::profiles::view(Vec::new(), language));
        }
        Ok(tray::profiles::view(self.desktop_profiles()?, language))
    }

    /// Aligns the linked instance in `config_dir` (absolute, or starting with `~`) with the
    /// primary instance, registered or not: its Global config keys, then its marketplaces and
    /// plugins, as [`Core::repair`] does for those items, and nothing else (no links, no
    /// launcher). For the add flow, before the instance is logged in and registered. Refuses
    /// while a Claude Code process uses the instance; a failed install is reported in its item.
    /// Aligning the primary instance changes nothing.
    pub fn align(&self, config_dir: &Path) -> Result<RepairReport, Error> {
        repair::align(
            &self.home,
            &self.paths(),
            self.env.as_ref(),
            config_dir,
            &|| self.launchers().ok(),
            &|| self.sync_launchers(),
        )
    }
}
