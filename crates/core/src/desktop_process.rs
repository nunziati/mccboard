//! Which desktop profiles run (spec #34, ticket #36), told from the processes the environment
//! port lists, as the session guard tells which instances Claude Code uses.
//!
//! The port lists every process named `Claude` with its command line ([`ProcessListing`]);
//! this module decides, in pure code tested on fixture command lines, which of them are the
//! desktop app and which data folder each uses:
//! - **the app** is `app\Claude.exe` inside the MSIX package (`…\WindowsApps\Claude_…`), or
//!   any program named `Claude` started with `--user-data-dir` (Claude Code's `claude.exe`
//!   never is). The old Squirrel install (`%LOCALAPPDATA%\AnthropicClaude`) is recognised too,
//!   so its folders are never deleted under it;
//! - **its data folder** is the value of `--user-data-dir=`: one argument, its value quoted or
//!   not. Helper processes (`--type=renderer`, `utility`, `gpu-process`, `crashpad-handler`)
//!   carry it too, so a profile whose only process left is a helper still runs;
//! - **the default desktop profile** runs when a process of the app has no `--user-data-dir`,
//!   or one naming the app's own data folder (`%APPDATA%\Claude`, or its copy virtualised in
//!   the package folder).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{desktop_app, paths, session_guard};

/// A process named `Claude` (`Claude.exe`) as the operating system lists it: what the
/// environment port returns, before the core decides whether it is the desktop app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessListing {
    pub pid: u32,
    /// The process that started it, when known.
    pub parent_pid: Option<u32>,
    /// The process name (`Claude.exe`).
    pub name: String,
    /// The program's full path; `None` when it can't be read.
    pub exe: Option<PathBuf>,
    /// The command line split into arguments, the program first. Empty when it can't be read
    /// (another user's process, an elevated one).
    pub args: Vec<String>,
}

/// A running process of the Claude desktop app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopProcess {
    pub pid: u32,
    /// The process that started it, when known: for a main process, mccboard when mccboard
    /// opened it.
    pub parent_pid: Option<u32>,
    /// The process name (`Claude.exe`).
    pub name: String,
    /// Chromium's `--type=` of a helper process (`renderer`, `gpu-process`, `utility`,
    /// `crashpad-handler`); `None` for the main process of a profile.
    pub helper: Option<String>,
    pub data_dir: DesktopDataDir,
}

impl DesktopProcess {
    /// Whether it is the main process of its profile, not one of its helpers.
    pub fn is_main(&self) -> bool {
        self.helper.is_none()
    }
}

/// The data directory a process of the desktop app uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopDataDir {
    /// Started without `--user-data-dir`: the app's own data directory (the default desktop
    /// profile).
    Default,
    /// Its `--user-data-dir`, absolute.
    Dir { path: PathBuf },
    /// Its command line can't be read (another user's process, an elevated one), or names a
    /// relative folder.
    Unknown,
}

const USER_DATA_DIR: &str = "--user-data-dir";
const TYPE: &str = "--type=";

/// The listed processes that are the desktop app, each with its data folder.
pub(crate) fn desktop_processes(listed: Vec<ProcessListing>) -> Vec<DesktopProcess> {
    listed.into_iter().filter_map(interpret).collect()
}

/// Whether `listing` is one of the desktop app's processes (its main process or a helper),
/// rather than Claude Code. The session guard leaves these out: the app's `Claude.exe` stays
/// open in the tray, and would otherwise hold its instance "in use" all day. The sessions of
/// its Code tab are Claude Code processes of their own and still count.
pub(crate) fn is_desktop_app(listing: &ProcessListing) -> bool {
    interpret(listing.clone()).is_some()
}

/// The desktop app's process `listing` is, or `None` for any other program.
pub(crate) fn interpret(listing: ProcessListing) -> Option<DesktopProcess> {
    let data_folder = user_data_dir(&listing.args);
    let installed = listing.exe.as_deref().is_some_and(is_app_program);
    let named_claude = stem(Path::new(&listing.name)) == "claude"
        || listing
            .exe
            .as_deref()
            .is_some_and(|exe| stem(exe) == "claude");
    let data_dir = match data_folder {
        Some(dir) if named_claude || installed => {
            if is_absolute(&dir) {
                DesktopDataDir::Dir {
                    path: paths::normalize(&dir),
                }
            } else {
                DesktopDataDir::Unknown
            }
        }
        Some(_) => return None,
        None if !installed => return None,
        None if listing.args.is_empty() => DesktopDataDir::Unknown,
        None => DesktopDataDir::Default,
    };
    Some(DesktopProcess {
        pid: listing.pid,
        parent_pid: listing.parent_pid,
        helper: helper_type(&listing.args),
        name: listing.name,
        data_dir,
    })
}

/// The processes among `processes` that run the profile whose data folder is `data_folder`.
/// One whose folder can't be told doesn't count.
pub(crate) fn using_folder(
    processes: &[DesktopProcess],
    home: &Path,
    data_folder: &Path,
) -> Vec<DesktopProcess> {
    let default_dirs = desktop_app::default_data_dirs(home);
    let is_default = default_dirs
        .iter()
        .any(|dir| session_guard::same_folder(dir, data_folder));
    sorted(processes.iter().filter(|p| match &p.data_dir {
        DesktopDataDir::Dir { path } => session_guard::same_folder(path, data_folder),
        DesktopDataDir::Default => is_default,
        DesktopDataDir::Unknown => false,
    }))
}

/// The processes among `processes` that run the default desktop profile: without
/// `--user-data-dir`, or with the app's own data folder.
pub(crate) fn running_default(processes: &[DesktopProcess], home: &Path) -> Vec<DesktopProcess> {
    let default_dirs = desktop_app::default_data_dirs(home);
    sorted(processes.iter().filter(|p| {
        match &p.data_dir {
            DesktopDataDir::Default => true,
            DesktopDataDir::Dir { path } => default_dirs
                .iter()
                .any(|dir| session_guard::same_folder(dir, path)),
            DesktopDataDir::Unknown => false,
        }
    }))
}

fn sorted<'a>(processes: impl Iterator<Item = &'a DesktopProcess>) -> Vec<DesktopProcess> {
    let mut out: Vec<DesktopProcess> = processes.cloned().collect();
    out.sort_by_key(|p| p.pid);
    out
}

/// Whether `exe` is the desktop app's program: `app\Claude.exe` in an MSIX package of the app
/// under `WindowsApps`, or `Claude.exe` in the old Squirrel install.
fn is_app_program(exe: &Path) -> bool {
    let exe = exe.to_string_lossy().replace('\\', "/").to_lowercase();
    let packaged = exe.ends_with("/app/claude.exe")
        && ["/windowsapps/claude_", "/windowsapps/anthropic.claude"]
            .iter()
            .any(|folder| exe.contains(folder));
    packaged || (exe.contains("/anthropicclaude/") && exe.ends_with("/claude.exe"))
}

/// The value of `--user-data-dir=<dir>` (Chromium's form), or of `--user-data-dir <dir>`; one
/// pair of surrounding quotes is dropped, as a command line split by hand would keep them.
pub(crate) fn user_data_dir(args: &[String]) -> Option<PathBuf> {
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        let Some(rest) = strip_flag(arg, USER_DATA_DIR) else {
            continue;
        };
        let value = match rest {
            "" => args.next().map(String::as_str),
            rest => rest.strip_prefix('='),
        };
        if let Some(value) = value.map(unquote).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

/// The value of `--type=`: set on every helper process, never on the main one.
fn helper_type(args: &[String]) -> Option<String> {
    args.iter()
        .skip(1)
        .find_map(|arg| strip_flag(arg, TYPE))
        .map(|value| unquote(value).to_owned())
        .filter(|value| !value.is_empty())
}

/// `arg` after `flag`, compared ASCII-case-insensitively.
fn strip_flag<'a>(arg: &'a str, flag: &str) -> Option<&'a str> {
    let head = arg.get(..flag.len())?;
    head.eq_ignore_ascii_case(flag).then(|| &arg[flag.len()..])
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn stem(path: &Path) -> String {
    // A Windows path read on another OS is one component: take what follows the last `\`.
    let name = path.to_string_lossy();
    let name = name.rsplit(['\\', '/']).next().unwrap_or_default();
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_lowercase()
}

/// Absolute on this OS, or a Windows path with a drive or a share (`C:\…`, `\\server\…`),
/// which fixtures use on any OS.
fn is_absolute(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    drive || text.starts_with(r"\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str =
        r"C:\Program Files\WindowsApps\Claude_1.0.2342.0_x64__pzs8sxrjxfjjc\app\Claude.exe";

    fn listing(pid: u32, exe: Option<&str>, args: &[&str]) -> ProcessListing {
        ProcessListing {
            pid,
            parent_pid: Some(1),
            name: "Claude.exe".into(),
            exe: exe.map(PathBuf::from),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    #[test]
    fn the_desktop_app_is_not_a_claude_code_session() {
        let code_tab_session = r"C:\Users\me\AppData\Roaming\Claude\claude-code\2.1.0\claude.exe";
        let native = r"C:\Users\me\.local\bin\claude.exe";
        let app = [
            listing(10, Some(EXE), &[EXE]),
            listing(11, Some(EXE), &[EXE, "--type=renderer"]),
            listing(
                12,
                Some(EXE),
                &[EXE, r"--user-data-dir=C:\Users\me\AppData\Local\Claude-B"],
            ),
            listing(
                13,
                None,
                &[
                    "Claude.exe",
                    r"--user-data-dir=C:\Users\me\AppData\Local\Claude-B",
                ],
            ),
        ];
        for process in &app {
            assert!(is_desktop_app(process), "{process:?}");
        }
        let claude_code = [
            listing(20, Some(native), &[native]),
            listing(21, Some(native), &[native, "--resume"]),
            listing(
                22,
                Some(code_tab_session),
                &[code_tab_session, "--output-format", "stream-json"],
            ),
            listing(23, None, &[]),
        ];
        for process in &claude_code {
            assert!(!is_desktop_app(process), "{process:?}");
        }
    }

    fn dir(path: &str) -> DesktopDataDir {
        DesktopDataDir::Dir {
            path: paths::normalize(Path::new(path)),
        }
    }

    #[test]
    fn an_unquoted_folder_is_read_from_the_main_process() {
        let p = interpret(listing(
            10,
            Some(EXE),
            &[EXE, r"--user-data-dir=C:\Users\me\AppData\Local\Claude-B"],
        ))
        .unwrap();
        assert_eq!(p.data_dir, dir(r"C:\Users\me\AppData\Local\Claude-B"));
        assert!(p.is_main());
        assert_eq!(p.parent_pid, Some(1));
    }

    #[test]
    fn a_quoted_folder_with_spaces_is_read_without_its_quotes() {
        let p = interpret(listing(
            10,
            Some(EXE),
            &[EXE, r#"--user-data-dir="C:\Users\me\My Profiles\Claude B""#],
        ))
        .unwrap();
        assert_eq!(p.data_dir, dir(r"C:\Users\me\My Profiles\Claude B"));
        // Split by Windows' rules, the quotes are gone already and the spaces stay.
        let p = interpret(listing(
            10,
            Some(EXE),
            &[EXE, r"--user-data-dir=C:\Users\me\My Profiles\Claude B"],
        ))
        .unwrap();
        assert_eq!(p.data_dir, dir(r"C:\Users\me\My Profiles\Claude B"));
    }

    #[test]
    fn renderer_and_utility_helpers_carry_their_profiles_folder() {
        for (kind, extra) in [
            ("renderer", "--app-path=C:\\x\\app.asar"),
            ("utility", "--utility-sub-type=network.mojom.NetworkService"),
            ("gpu-process", "--gpu-preferences=AAA"),
            ("crashpad-handler", "--monitor-self"),
        ] {
            let p = interpret(listing(
                11,
                Some(EXE),
                &[
                    EXE,
                    &format!("--type={kind}"),
                    r"--user-data-dir=C:\Users\me\AppData\Local\Claude-B",
                    extra,
                ],
            ))
            .unwrap();
            assert_eq!(p.helper.as_deref(), Some(kind));
            assert!(!p.is_main());
            assert_eq!(p.data_dir, dir(r"C:\Users\me\AppData\Local\Claude-B"));
        }
    }

    #[test]
    fn the_default_profiles_main_process_has_no_folder() {
        let p = interpret(listing(12, Some(EXE), &[EXE])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Default);
        assert!(p.is_main());
        let p = interpret(listing(12, Some(EXE), &[EXE, "--squirrel-firstrun"])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Default);
    }

    #[test]
    fn the_flags_are_matched_whatever_their_case() {
        let p = interpret(listing(
            13,
            Some(EXE),
            &[EXE, "--TYPE=renderer", r"--User-Data-Dir=C:\p\b"],
        ))
        .unwrap();
        assert_eq!(p.helper.as_deref(), Some("renderer"));
        assert_eq!(p.data_dir, dir(r"C:\p\b"));
    }

    #[test]
    fn an_unreadable_command_line_of_the_app_is_unknown() {
        let p = interpret(listing(14, Some(EXE), &[])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Unknown);
    }

    #[test]
    fn a_relative_folder_is_unknown() {
        let p = interpret(listing(15, Some(EXE), &[EXE, "--user-data-dir=Claude-B"])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Unknown);
    }

    #[test]
    fn claude_code_and_other_programs_are_not_the_app() {
        let code = r"C:\Users\me\.local\bin\claude.exe";
        assert_eq!(interpret(listing(20, Some(code), &[code])), None);
        assert_eq!(
            interpret(listing(21, Some(code), &[code, "--resume"])),
            None
        );
        assert_eq!(interpret(listing(22, None, &["claude"])), None);
        let mut other = listing(
            23,
            Some(r"C:\x\Code.exe"),
            &["Code.exe", "--user-data-dir=C:\\x"],
        );
        other.name = "Code.exe".into();
        assert_eq!(interpret(other), None);
        // `app\Claude.exe` outside the package, without a folder: not told apart from others.
        let portable = r"C:\Tools\Claude\app\Claude.exe";
        assert_eq!(interpret(listing(24, Some(portable), &[portable])), None);
    }

    #[test]
    fn a_program_named_claude_with_a_folder_is_the_app_wherever_it_runs_from() {
        let p = interpret(listing(
            25,
            None,
            &["Claude.exe", r"--user-data-dir=C:\p\b"],
        ))
        .unwrap();
        assert_eq!(p.data_dir, dir(r"C:\p\b"));
    }

    #[test]
    fn the_old_squirrel_install_and_package_spellings_are_recognised() {
        let squirrel = r"C:\Users\me\AppData\Local\AnthropicClaude\app-0.9.0\claude.exe";
        let p = interpret(listing(26, Some(squirrel), &[squirrel])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Default);
        let lower =
            r"c:\program files\windowsapps\claude_2.0.0.0_arm64__pzs8sxrjxfjjc\app\claude.exe";
        let p = interpret(listing(27, Some(lower), &[lower])).unwrap();
        assert_eq!(p.data_dir, DesktopDataDir::Default);
    }

    #[test]
    fn the_folder_also_reads_from_the_two_argument_spelling() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            user_data_dir(&args(&[
                "Claude.exe",
                "--type=renderer",
                "--user-data-dir",
                "/p/b"
            ])),
            Some(PathBuf::from("/p/b"))
        );
        assert_eq!(
            user_data_dir(&args(&["Claude.exe", "--user-data-dir"])),
            None
        );
        assert_eq!(
            user_data_dir(&args(&["Claude.exe", "--user-data-dir="])),
            None
        );
        assert_eq!(
            user_data_dir(&args(&["Claude.exe", "--user-data-directory=/x"])),
            None
        );
        // The program itself is never read as a flag.
        assert_eq!(user_data_dir(&args(&["--user-data-dir=/x"])), None);
    }

    /// A home that is absolute on the OS the tests run on.
    fn home() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            r"C:\Users\me"
        } else {
            "/home/me"
        })
    }

    fn data_dir_arg(path: &Path) -> String {
        format!("--user-data-dir={}", path.display())
    }

    #[test]
    fn the_default_profile_runs_without_a_folder_or_with_the_apps_own() {
        let home = home();
        let own = home.join("AppData").join("Roaming").join("Claude");
        let b = home.join("profiles").join("b");
        let processes = desktop_processes(vec![
            listing(30, Some(EXE), &[EXE]),
            listing(
                31,
                Some(EXE),
                &[EXE, "--type=renderer", &data_dir_arg(&own)],
            ),
            listing(32, Some(EXE), &[EXE, &data_dir_arg(&b)]),
            listing(33, Some(EXE), &[]),
        ]);
        let pids = |ps: Vec<DesktopProcess>| ps.iter().map(|p| p.pid).collect::<Vec<_>>();
        assert_eq!(pids(running_default(&processes, &home)), [30, 31]);
        assert_eq!(pids(using_folder(&processes, &home, &b)), [32]);
        assert_eq!(pids(using_folder(&processes, &home, &own)), [30, 31]);
    }

    #[test]
    fn a_profile_whose_only_process_left_is_a_helper_still_runs() {
        let home = home();
        let b = home.join("profiles").join("b");
        let processes = desktop_processes(vec![listing(
            40,
            Some(EXE),
            &[EXE, "--type=utility", &data_dir_arg(&b)],
        )]);
        let running = using_folder(&processes, &home, &b);
        assert_eq!(running.len(), 1);
        assert!(!running[0].is_main());
    }
}
