//! A desktop profile's own identity on the Windows taskbar (spec #34, ticket #37, ADR 0008):
//! its button, its colored icon and its pinnable Start-menu shortcut. Every profile has one
//! but the default desktop profile, which stays the app as it is.
//!
//! - **AppUserModelID** ([`app_user_model_id`]): `io.github.nunziati.mccboard.profile.<account
//!   uuid>`. Windows groups windows and pins by it. It comes from the account, not the label,
//!   so a rename keeps pins and grouping.
//! - **Windows** of a running profile are tagged from outside, in mccboard's app: the
//!   AppUserModelID, the relaunch command (mccboard with [`OPEN_PROFILE_ARG`]), the display
//!   name `Claude <label>` and the icon. [`running`] says which processes belong to which
//!   profile and with what identity; the Windows code that applies it lives in
//!   `windows_taskbar`.
//! - **The shortcut** `Claude <label>.lnk` in the Start menu's Programs folder starts mccboard
//!   with `--open-profile <uuid>` and carries the profile's AppUserModelID and icon, so a
//!   pinned shortcut and the profile's windows share one button. [`sync`] makes the shortcuts
//!   match the stored profiles: written when a profile is added, rewritten when its label or
//!   color changes, removed with it. Pinning stays a manual right-click: Windows doesn't allow
//!   it from code.
//! - **Which mccboard they start**: the running app, except that a development build (in a
//!   cargo build folder) leaves alone a shortcut that starts an installed copy still there.
//!   With both on one machine, the installed app keeps the shortcuts and pins.
//! - **Which shortcuts are mccboard's**: the ones it recorded in
//!   `~/.mccboard/state/shortcuts.json`. Only those are ever rewritten or removed; a file of the
//!   same name it didn't make is left alone.
//! - **Icons** come from the [`IconCache`]: the package's logo recolored, or the app's own icon
//!   when the package has no logo. A profile whose icon can't be drawn still gets its shortcut,
//!   with mccboard's icon, and a warning.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::account::{self, AccountUuid, Color};
use crate::config::{Config, ConfigFile, DesktopProfileEntry};
use crate::{
    Environment, Error, IconCache, IconSources, OsKind, Paths, ProfileIcons, atomic_file,
    desktop_launch, desktop_process, paths,
};

/// The argument that makes mccboard's app open a desktop profile: `--open-profile <uuid>`.
pub const OPEN_PROFILE_ARG: &str = "--open-profile";

const APP_USER_MODEL_ID_PREFIX: &str = "io.github.nunziati.mccboard.profile.";

/// mccboard's record of the shortcuts it wrote, under `~/.mccboard/state/`.
const RECORD_FILE: &str = "shortcuts.json";

/// The AppUserModelID of `account`'s desktop profile. At most 128 characters and no spaces,
/// as Windows requires, for any real account UUID.
pub fn app_user_model_id(account: &AccountUuid) -> String {
    format!("{APP_USER_MODEL_ID_PREFIX}{}", account.as_str())
}

/// The arguments that make mccboard's app open `account`'s desktop profile.
pub fn open_profile_args(account: &AccountUuid) -> Vec<String> {
    vec![OPEN_PROFILE_ARG.to_owned(), account.as_str().to_owned()]
}

/// What mccboard's app was started for, from its command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppArgs {
    /// `--open-profile <uuid>` (or `--open-profile=<uuid>`): open that account's desktop
    /// profile, from its Start-menu shortcut or a relaunch from the taskbar.
    pub open_profile: Option<AccountUuid>,
}

impl AppArgs {
    /// Reads `argv`, the program first. Arguments it doesn't know are ignored; the last
    /// `--open-profile` wins.
    pub fn parse<I, S>(argv: I) -> AppArgs
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut parsed = AppArgs::default();
        let mut args = argv.into_iter().skip(1);
        while let Some(arg) = args.next() {
            let arg = arg.as_ref();
            let value = if arg == OPEN_PROFILE_ARG {
                args.next().map(|v| v.as_ref().to_owned())
            } else if let Some(value) = arg
                .strip_prefix(OPEN_PROFILE_ARG)
                .and_then(|rest| rest.strip_prefix('='))
            {
                Some(value.to_owned())
            } else {
                continue;
            };
            if let Some(value) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
                parsed.open_profile = Some(AccountUuid::new(value));
            }
        }
        parsed
    }
}

/// A Start-menu shortcut, as the core asks the environment port to write it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// `<Start menu>\Programs\Claude <label>.lnk`.
    pub path: PathBuf,
    /// mccboard's app.
    pub target: PathBuf,
    /// `--open-profile <uuid>`, quoted for a Windows command line.
    pub arguments: String,
    /// The profile's `.ico`; `None`: the target's own icon.
    pub icon: Option<PathBuf>,
    /// The profile's AppUserModelID, set on the shortcut so its pin groups the profile's
    /// windows.
    pub app_user_model_id: String,
    /// The shortcut's tooltip: `Claude <label>`.
    pub description: String,
}

/// What [`sync`] did to the Start-menu shortcuts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ShortcutSync {
    /// Shortcuts written: new, renamed, or with a new icon or target.
    pub written: Vec<PathBuf>,
    /// Shortcuts already as they should be.
    pub unchanged: Vec<PathBuf>,
    /// Shortcuts mccboard made that no profile needs any more: deleted.
    pub removed: Vec<PathBuf>,
    pub warnings: Vec<ShortcutWarning>,
}

/// A change to the accounts or desktop profiles, saved in `config.toml`, and whether the
/// Start-menu shortcuts followed it. The change stands either way: shortcuts that couldn't
/// follow are a warning for front ends to show, never a failed operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Saved<T> {
    /// What the operation returns: the account, the profile, the removal.
    #[serde(flatten)]
    pub value: T,
    /// Why the shortcuts couldn't be made to match ([`crate::Core::sync_desktop_shortcuts`]);
    /// `None` when they match. Running that sync again, or the app's next start, retries.
    pub shortcuts_not_synced: Option<ShortcutsNotSynced>,
}

impl<T> Saved<T> {
    pub(crate) fn new(value: T, sync: Result<ShortcutSync, Error>) -> Self {
        Saved {
            value,
            shortcuts_not_synced: sync.err().map(|error| ShortcutsNotSynced::from(&error)),
        }
    }
}

/// The error a shortcut sync stopped on, as data front ends show and translate by `code`, as
/// they do an error ([`Error::code`], [`Error::path`], [`Error::detail`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShortcutsNotSynced {
    pub code: &'static str,
    pub path: Option<PathBuf>,
    pub detail: Option<String>,
    /// The error's English message, for the CLI.
    pub message: String,
}

impl From<&Error> for ShortcutsNotSynced {
    fn from(error: &Error) -> Self {
        ShortcutsNotSynced {
            code: error.code(),
            path: error.path().map(Path::to_path_buf),
            detail: error.detail().map(str::to_owned),
            message: error.to_string(),
        }
    }
}

/// Something about the shortcuts the user may want to know; none of them stops the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShortcutWarning {
    /// The profile's colored icon couldn't be drawn (`error` is the [`Error::code`] why): its
    /// shortcut shows mccboard's icon until it can.
    NoIcon { account: AccountUuid, error: String },
    /// A file mccboard didn't make already has the shortcut's name: it is left as it is, and
    /// the profile has no shortcut.
    NameTaken { path: PathBuf },
    /// A shortcut mccboard made, which no profile needs any more, couldn't be deleted. It is
    /// tried again next time.
    NotRemoved { path: PathBuf, reason: String },
    /// mccboard's record of its shortcuts couldn't be saved: a shortcut it made may stay
    /// behind when its profile goes.
    RecordNotSaved { path: PathBuf, reason: String },
}

/// What a running desktop profile's windows are tagged with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowIdentity {
    /// [`app_user_model_id`]: the window joins the profile's button and pin.
    pub app_user_model_id: String,
    /// `Claude <label>`: the name a pin made from the window shows.
    pub display_name: String,
    /// mccboard's app with `--open-profile <uuid>`, quoted: what a pin made from the window
    /// starts. `None` when mccboard's app can't be found; the window is tagged without it.
    pub relaunch_command: Option<String>,
    /// The profile's `.ico`, for the window and a pin made from it; `None` when it can't be
    /// drawn (the window keeps the app's icon).
    pub icon: Option<PathBuf>,
}

/// A desktop profile that runs now, with the processes whose windows get its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunningProfile {
    pub account: AccountUuid,
    pub label: String,
    pub color: Color,
    /// Every process of the app running it, the main one and its helpers, by pid. Electron
    /// opens its windows in the main process; helpers are listed too, which costs nothing.
    pub pids: Vec<u32>,
    /// The main one among `pids` (the one without Chromium's `--type=`), which owns the app's
    /// notification-area icon; `None` when none of them can be told to be it.
    pub main_pid: Option<u32>,
    /// Its cached icons; `None` when they can't be drawn.
    pub icons: Option<ProfileIcons>,
    pub identity: WindowIdentity,
}

/// One shortcut mccboard wrote, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    account: AccountUuid,
    shortcut: Shortcut,
}

/// A stored desktop profile whose account is registered: it gets a taskbar identity.
struct Profile<'a> {
    entry: &'a DesktopProfileEntry,
    label: String,
    color: Color,
}

/// The desktop profiles that run now, each with its processes and window identity; icons are
/// drawn and cached as needed. The default desktop profile is never among them.
pub(crate) fn running(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
) -> Result<Vec<RunningProfile>, Error> {
    let config = ConfigFile::load(&paths.config_file())?.config;
    let processes = desktop_process::desktop_processes(env.desktop_processes());
    if processes.is_empty() {
        return Ok(Vec::new());
    }
    let recorded = load_record(&paths.state_dir().join(RECORD_FILE));
    let target = env
        .mccboard_app()
        .ok()
        .map(|app| shortcut_target(app, &recorded));
    let mut running = Vec::new();
    for profile in profiles(home, &config) {
        let account = &profile.entry.account;
        let using = desktop_process::using_folder(&processes, home, &profile.entry.data_folder);
        let pids: Vec<u32> = using.iter().map(|p| p.pid).collect();
        let main_pid = using.iter().find(|p| p.is_main()).map(|p| p.pid);
        if pids.is_empty() {
            continue;
        }
        let icons = ensure_icons(home, paths, env, account, &profile.color).ok();
        let identity = WindowIdentity {
            app_user_model_id: app_user_model_id(account),
            display_name: display_name(&profile.label),
            relaunch_command: target.as_deref().map(|target| {
                let mut line = quote(&target.to_string_lossy());
                line.push(' ');
                line.push_str(&command_line(&open_profile_args(account)));
                line
            }),
            icon: icons.as_ref().map(|i| i.app.clone()),
        };
        running.push(RunningProfile {
            account: account.clone(),
            label: profile.label,
            color: profile.color,
            pids,
            main_pid,
            icons,
            identity,
        });
    }
    Ok(running)
}

/// Makes the Start-menu shortcuts match the stored desktop profiles (Windows; elsewhere it
/// does nothing). Writes a missing or outdated shortcut, deletes the ones mccboard made that no
/// profile needs, and leaves the rest. With no profile and no shortcut recorded, it asks the
/// system nothing; in another home than the user's (`mccboard --home`) it does nothing.
///
/// Fails, before writing anything, when the Start menu or mccboard's app can't be found; after
/// the others are done, when a shortcut can't be written.
pub(crate) fn sync(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
) -> Result<ShortcutSync, Error> {
    let mut report = ShortcutSync::default();
    if env.os() != OsKind::Windows {
        return Ok(report);
    }
    let config = ConfigFile::load(&paths.config_file())?.config;
    let record_path = paths.state_dir().join(RECORD_FILE);
    let recorded = load_record(&record_path);
    let profiles = profiles(home, &config);
    if profiles.is_empty() && recorded.is_empty() {
        return Ok(report);
    }

    let mut wanted = Vec::new();
    if !profiles.is_empty() {
        let Some(folder) = env
            .start_menu_programs(home)
            .map_err(|e| Error::StartMenuNotFound {
                reason: e.to_string(),
            })?
        else {
            // Another home than the user's: no Start menu to keep in step.
            return Ok(report);
        };
        let app = env.mccboard_app().map_err(|e| Error::AppProgramNotFound {
            reason: e.to_string(),
        })?;
        let target = shortcut_target(app, &recorded);
        let mut names = HashSet::new();
        for profile in &profiles {
            let account = &profile.entry.account;
            let icon = match ensure_icons(home, paths, env, account, &profile.color) {
                Ok(icons) => Some(icons.app),
                Err(error) => {
                    report.warnings.push(ShortcutWarning::NoIcon {
                        account: account.clone(),
                        error: error.code().to_owned(),
                    });
                    None
                }
            };
            let name = unique_name(&shortcut_file_name(&profile.label), &mut names);
            wanted.push(Record {
                account: account.clone(),
                shortcut: Shortcut {
                    path: folder.join(name),
                    target: target.clone(),
                    arguments: command_line(&open_profile_args(account)),
                    icon,
                    app_user_model_id: app_user_model_id(account),
                    description: display_name(&profile.label),
                },
            });
        }
    }

    let recorded_at = |path: &Path| {
        recorded
            .iter()
            .find(|r| paths::same(&r.shortcut.path, path))
    };
    let mut kept: Vec<Record> = Vec::new();
    let mut failure = None;
    for record in wanted {
        let path = record.shortcut.path.clone();
        let previous = recorded_at(&path);
        let exists = fs::symlink_metadata(&path).is_ok();
        if exists && previous.is_none() {
            report.warnings.push(ShortcutWarning::NameTaken { path });
            continue;
        }
        if exists && previous == Some(&record) {
            report.unchanged.push(path);
            kept.push(record);
            continue;
        }
        match env.write_shortcut(&record.shortcut) {
            Ok(()) => {
                report.written.push(path);
                kept.push(record);
            }
            Err(e) => {
                // The old shortcut, if any, stays mccboard's: it is retried or removed later.
                kept.extend(previous.cloned());
                failure.get_or_insert(Error::ShortcutNotWritten {
                    path,
                    reason: e.to_string(),
                });
            }
        }
    }
    for old in &recorded {
        let path = &old.shortcut.path;
        if kept.iter().any(|k| paths::same(&k.shortcut.path, path)) {
            continue;
        }
        match fs::remove_file(path) {
            Ok(()) => report.removed.push(path.clone()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                report.warnings.push(ShortcutWarning::NotRemoved {
                    path: path.clone(),
                    reason: e.to_string(),
                });
                kept.push(old.clone());
            }
        }
    }

    if kept != recorded
        && let Err(e) = save_record(&record_path, &kept)
    {
        report.warnings.push(ShortcutWarning::RecordNotSaved {
            path: record_path,
            reason: e.to_string(),
        });
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(report),
    }
}

/// The mccboard that shortcuts and relaunch commands start: `app`, the running one, unless it
/// is a development build and a shortcut mccboard made starts an installed copy that is still
/// there. The development build takes the shortcuts only once that copy is gone.
fn shortcut_target(app: PathBuf, recorded: &[Record]) -> PathBuf {
    if !is_cargo_build(&app) {
        return app;
    }
    recorded
        .iter()
        .map(|r| &r.shortcut.target)
        .find(|target| !is_cargo_build(target) && target.is_file())
        .cloned()
        .unwrap_or(app)
}

/// Whether `program` lies in a cargo build folder, which cargo marks with a `CACHEDIR.TAG`.
fn is_cargo_build(program: &Path) -> bool {
    program
        .ancestors()
        .skip(1)
        .any(|dir| dir.join("CACHEDIR.TAG").is_file())
}

/// The stored desktop profiles whose account is registered, by label. A hand-edited entry for
/// the default profile's account is left out: that profile keeps the app's own identity.
fn profiles<'a>(home: &Path, config: &'a Config) -> Vec<Profile<'a>> {
    let default = config.default_profile_account(home);
    let mut profiles: Vec<Profile<'a>> = config
        .desktop_profiles
        .iter()
        .filter(|entry| Some(&entry.account) != default)
        .filter_map(|entry| {
            let account = config.account(&entry.account)?;
            let account = config.shown_account(home, account);
            Some(Profile {
                entry,
                label: account.label,
                color: account.color,
            })
        })
        .collect();
    profiles.sort_by(|a, b| {
        account::by_label((&a.label, &a.entry.account), (&b.label, &b.entry.account))
    });
    profiles
}

/// `account`'s icons in `color`, drawn when missing from the installed app's logos, else its
/// program's icon.
fn ensure_icons(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    account: &AccountUuid,
    color: &Color,
) -> Result<ProfileIcons, Error> {
    IconCache::new(paths).ensure(account, color, || {
        let program = desktop_launch::find_app(home, env)?;
        let install_dir = program
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::NoIconSource)?;
        let mut sources = IconSources::from_package(install_dir)?;
        if sources.plated.is_none() && sources.unplated.is_none() {
            sources.plated = env.program_icon(&program).ok();
        }
        Ok(sources)
    })
}

/// `Claude <label>`: the shortcut's name and tooltip, and a pin's display name.
fn display_name(label: &str) -> String {
    format!("Claude {label}")
}

/// `Claude <label>.lnk`, with the characters a Windows file name can't hold replaced by `_`
/// and no trailing dot or space before the extension.
pub fn shortcut_file_name(label: &str) -> String {
    let name: String = display_name(label)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{}.lnk", name.trim_end_matches(['.', ' ']))
}

/// `name`, or `name (2).lnk`, `(3)`… when another shortcut of this sync has it already
/// (labels that differ only in characters a file name can't hold, or in case).
fn unique_name(name: &str, taken: &mut HashSet<String>) -> String {
    let stem = name.strip_suffix(".lnk").unwrap_or(name);
    let mut candidate = name.to_owned();
    let mut n = 2;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = format!("{stem} ({n}).lnk");
        n += 1;
    }
    candidate
}

/// `args` as one Windows command line, each quoted as `CommandLineToArgvW` reads it back.
pub fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One argument of a Windows command line: as it is when it holds no space, tab or quote;
/// otherwise in quotes, with quotes and the backslashes before them escaped.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The shortcuts mccboard recorded; none when the record is missing or unreadable (mccboard's
/// own state: then it can't tell which shortcuts are its own, and touches none).
fn load_record(path: &Path) -> Vec<Record> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_record(path: &Path, records: &[Record]) -> io::Result<()> {
    let text = serde_json::to_string_pretty(records).map_err(io::Error::other)?;
    atomic_file::write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "11111111-1111-4111-8111-111111111111";

    fn args(list: &[&str]) -> AppArgs {
        AppArgs::parse(list.iter().copied())
    }

    #[test]
    fn the_app_user_model_id_comes_from_the_account_uuid() {
        let id = app_user_model_id(&AccountUuid::new(UUID));
        assert_eq!(id, format!("io.github.nunziati.mccboard.profile.{UUID}"));
        assert!(id.len() <= 128);
        assert!(!id.contains(' '));
    }

    #[test]
    fn open_profile_is_read_from_the_command_line() {
        let uuid = Some(AccountUuid::new(UUID));
        assert_eq!(
            args(&["mccboard.exe", "--open-profile", UUID]).open_profile,
            uuid
        );
        assert_eq!(
            args(&["mccboard.exe", &format!("--open-profile={UUID}")]).open_profile,
            uuid
        );
        assert_eq!(
            args(&["mccboard.exe", "--other", "--open-profile", UUID, "x"]).open_profile,
            uuid
        );
        assert_eq!(args(&["mccboard.exe"]), AppArgs::default());
        assert_eq!(
            args(&["mccboard.exe", "--open-profile"]),
            AppArgs::default()
        );
        assert_eq!(
            args(&["mccboard.exe", "--open-profile="]),
            AppArgs::default()
        );
        assert_eq!(
            args(&["mccboard.exe", "--open-profiles", UUID]),
            AppArgs::default()
        );
        // argv[0] is the program, never an argument.
        assert_eq!(args(&["--open-profile"]), AppArgs::default());
        // The arguments a shortcut passes read back as the same profile.
        let mut argv = vec!["mccboard.exe".to_owned()];
        argv.extend(open_profile_args(&AccountUuid::new(UUID)));
        assert_eq!(AppArgs::parse(&argv).open_profile, uuid);
    }

    #[test]
    fn shortcut_names_hold_no_character_windows_refuses() {
        assert_eq!(shortcut_file_name("B"), "Claude B.lnk");
        assert_eq!(
            shortcut_file_name("Personal Max"),
            "Claude Personal Max.lnk"
        );
        assert_eq!(shortcut_file_name("a/b:c*?"), "Claude a_b_c__.lnk");
        assert_eq!(shortcut_file_name("dots..."), "Claude dots.lnk");
        let mut taken = HashSet::new();
        assert_eq!(unique_name("Claude a_.lnk", &mut taken), "Claude a_.lnk");
        assert_eq!(
            unique_name("Claude A_.lnk", &mut taken),
            "Claude A_ (2).lnk"
        );
        assert_eq!(
            unique_name("Claude a_.lnk", &mut taken),
            "Claude a_ (3).lnk"
        );
    }

    #[test]
    fn arguments_are_quoted_as_windows_reads_them() {
        let line =
            |list: &[&str]| command_line(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            line(&["--open-profile", UUID]),
            format!("--open-profile {UUID}")
        );
        assert_eq!(
            line(&[r"C:\Program Files\mccboard\mccboard.exe"]),
            r#""C:\Program Files\mccboard\mccboard.exe""#
        );
        assert_eq!(line(&[""]), r#""""#);
        assert_eq!(line(&[r#"a "b""#]), r#""a \"b\"""#);
        assert_eq!(line(&[r"C:\a b\"]), r#""C:\a b\\""#);
    }
}
