//! mccboard's launcher program on Windows, the shim. mccboard copies it into its bin folder
//! (`~/.mccboard/bin`) once per launcher, as `claude-b.exe`, next to a sidecar file,
//! `claude-b.shim`, that names the instance. Run as `claude-b`, it starts Claude Code in that
//! instance: `CLAUDE_CONFIG_DIR` set to the instance's folder, every argument passed on as
//! received (no shell re-quoting them), the console and standard streams shared, and
//! `claude`'s exit code returned.
//!
//! This library holds what both sides share and what can be tested on every OS: the sidecar's
//! format, which the core writes and the shim reads, and how the shim finds `claude`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The shim's program name, as built (`mccboard-shim.exe` on Windows). mccboard ships it next
/// to its own programs.
pub const PROGRAM_NAME: &str = "mccboard-shim";

/// The sidecar file: which instance a launcher starts Claude Code in.
pub mod sidecar {
    use std::path::{Path, PathBuf};

    /// The sidecar's extension: `claude-b.shim` sits next to `claude-b.exe`.
    pub const EXTENSION: &str = "shim";

    /// Marks the sidecars mccboard wrote; only their launchers are ever removed.
    pub const MARKER: &str = "Managed by mccboard";

    const KEY: &str = "CLAUDE_CONFIG_DIR=";

    /// The sidecar of a launcher starting Claude Code in `config_dir`: a comment line with the
    /// marker, then `CLAUDE_CONFIG_DIR=<folder>`.
    pub fn render(config_dir: &Path) -> String {
        format!(
            "# {MARKER}: the launcher next to this file starts Claude Code in the instance \
             below. Changes here are overwritten.\r\n{KEY}{}\r\n",
            config_dir.to_string_lossy()
        )
    }

    /// The config directory a sidecar names, if it names one.
    pub fn parse(text: &str) -> Option<PathBuf> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        text.lines()
            .find_map(|line| line.strip_prefix(KEY))
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    }

    /// Whether mccboard wrote the sidecar.
    pub fn is_managed(text: &str) -> bool {
        text.contains(MARKER)
    }
}

/// The extensions Windows tries when none is given, without `PATHEXT`.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// The extensions a process can be started from: programs, and batch files through cmd.exe.
/// Others a `PATHEXT` may list (`.ps1`, `.js`, `.vbs`…) need a host the shim doesn't pick.
const RUNNABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// Finds `program` on `path` (a `PATH` value) as a Windows terminal would: each folder in
/// order, and in each the extensions of `pathext` in order (`.COM;.EXE;.BAT;.CMD` when unset),
/// keeping only the runnable ones. The bare name is never a match: npm also installs a
/// `claude` shell script for Git Bash, which Windows can't start. Folders that are `skip`
/// (the shim's own, where the launchers are) are passed over, so a launcher never finds
/// itself or another launcher.
pub fn find_program(
    program: &str,
    path: &OsStr,
    pathext: Option<&OsStr>,
    skip: Option<&Path>,
) -> Option<PathBuf> {
    let pathext = pathext
        .and_then(OsStr::to_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_PATHEXT);
    let extensions: Vec<String> = pathext
        .split(';')
        .map(|ext| ext.trim().to_ascii_lowercase())
        .filter(|ext| RUNNABLE.contains(&ext.as_str()))
        .collect();
    let skip = skip.map(resolved);
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| skip.as_ref() != Some(&resolved(dir)))
        .find_map(|dir| {
            extensions
                .iter()
                .map(|ext| dir.join(format!("{program}{ext}")))
                .find(|candidate| candidate.is_file())
        })
}

/// A folder as it really is (case, `.`, trailing separators, links resolved), so two ways of
/// writing it compare equal; as written when it doesn't exist.
fn resolved(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

/// Whether `program` is a batch file, which Windows runs through cmd.exe.
pub fn is_batch_file(program: &Path) -> bool {
    program
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;

    #[test]
    fn a_sidecar_round_trips_its_folder() {
        for dir in [
            r"C:\Users\Giacomo Rossi\.claude-b",
            r"C:\Users\x\it's 100% b & c ^!",
            "/home/x/.claude-b",
            "C:\\Users\\Ünïcødé\\.claude-b",
        ] {
            let text = sidecar::render(Path::new(dir));
            assert!(sidecar::is_managed(&text));
            assert_eq!(sidecar::parse(&text), Some(PathBuf::from(dir)), "{text}");
        }
    }

    #[test]
    fn a_sidecar_without_a_folder_names_none() {
        assert_eq!(sidecar::parse(""), None);
        assert_eq!(sidecar::parse("# Managed by mccboard\r\n"), None);
        assert_eq!(sidecar::parse("CLAUDE_CONFIG_DIR=\r\n"), None);
        // Edited by hand in Notepad, which may add a byte order mark and drop the comment.
        assert_eq!(
            sidecar::parse("\u{feff}CLAUDE_CONFIG_DIR=D:\\b\n"),
            Some(PathBuf::from("D:\\b"))
        );
        assert!(!sidecar::is_managed("CLAUDE_CONFIG_DIR=D:\\b"));
    }

    struct Dirs {
        _root: tempfile::TempDir,
        dirs: Vec<PathBuf>,
    }

    /// Folders `0`, `1`, … in a temporary directory.
    fn dirs(n: usize) -> Dirs {
        let root = tempfile::tempdir().unwrap();
        let dirs = (0..n)
            .map(|i| {
                let dir = root.path().join(i.to_string());
                fs::create_dir(&dir).unwrap();
                dir
            })
            .collect();
        Dirs { _root: root, dirs }
    }

    fn path_of(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).unwrap()
    }

    fn touch(path: PathBuf) -> PathBuf {
        fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn claude_is_found_in_path_order_with_pathext_order_per_folder() {
        let d = dirs(3);
        let npm = touch(d.dirs[1].join("claude.cmd"));
        touch(d.dirs[1].join("claude"));
        touch(d.dirs[2].join("claude.exe"));
        let path = path_of(&[&d.dirs[0], &d.dirs[1], &d.dirs[2]]);

        assert_eq!(find_program("claude", &path, None, None), Some(npm.clone()));
        // Within one folder, PATHEXT decides.
        let both = touch(d.dirs[1].join("claude.exe"));
        assert_eq!(
            find_program("claude", &path, Some(OsStr::new(".CMD;.EXE")), None),
            Some(npm)
        );
        assert_eq!(find_program("claude", &path, None, None), Some(both));
    }

    #[test]
    fn the_bare_name_and_unrunnable_extensions_are_never_found() {
        let d = dirs(1);
        touch(d.dirs[0].join("claude"));
        touch(d.dirs[0].join("claude.ps1"));
        touch(d.dirs[0].join("claude.js"));
        let path = path_of(&[&d.dirs[0]]);

        assert_eq!(
            find_program("claude", &path, Some(OsStr::new(".PS1;.JS;.EXE")), None),
            None
        );
        // An empty PATHEXT falls back to Windows' default.
        let bat = touch(d.dirs[0].join("claude.bat"));
        assert_eq!(
            find_program("claude", &path, Some(OsStr::new("")), None),
            Some(bat)
        );
    }

    #[test]
    fn the_shims_own_folder_is_skipped_however_written() {
        let d = dirs(2);
        let bin = &d.dirs[0];
        touch(bin.join("claude.exe"));
        let real = touch(d.dirs[1].join("claude.exe"));
        // The same folder written another way: with `.` and a trailing separator.
        let other_spelling = bin.join(".").join("");
        let path = path_of(&[&other_spelling, &d.dirs[1]]);

        assert_eq!(
            find_program("claude", &path, None, Some(bin)),
            Some(real.clone())
        );
        assert_eq!(
            find_program("claude", &path_of(&[bin]), None, Some(bin)),
            None
        );
        assert_ne!(find_program("claude", &path, None, None), Some(real));
    }

    #[test]
    fn batch_files_are_told_apart() {
        assert!(is_batch_file(Path::new(r"C:\npm\claude.cmd")));
        assert!(is_batch_file(Path::new(r"C:\x\CLAUDE.BAT")));
        assert!(!is_batch_file(Path::new(r"C:\x\claude.exe")));
        assert!(!is_batch_file(Path::new(r"C:\x\claude")));
    }
}
