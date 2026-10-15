//! The built shim, run as a launcher against a stand-in `claude` that prints what it got
//! (Windows only; elsewhere the launchers are shell aliases and this checks nothing).
//!
//! Not a libtest harness: this test program is also the stand-in. Copied as `claude.exe`, it
//! prints `CLAUDE_CONFIG_DIR` and each argument (Rust's `Debug` form, so every character
//! shows), then exits with `STAND_IN_EXIT` (0 when unset).

use std::env;
use std::process;

fn main() {
    let me = env::current_exe().expect("the test program's path");
    if me
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("claude"))
    {
        stand_in();
    }
    #[cfg(windows)]
    windows::run();
    #[cfg(not(windows))]
    println!("the shim is a Windows program: nothing to run here");
}

/// Plays `claude`.
fn stand_in() -> ! {
    println!("dir={:?}", env::var_os("CLAUDE_CONFIG_DIR"));
    for arg in env::args_os().skip(1) {
        println!("arg={arg:?}");
    }
    let code = env::var("STAND_IN_EXIT")
        .ok()
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    process::exit(code)
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::fs;
    use std::os::windows::process::CommandExt;
    use std::panic;
    use std::path::{Path, PathBuf};
    use std::process::{self, Command, Output};

    use mccboard_shim::sidecar;

    /// Arguments that a shell re-quoting them would change.
    const ARGS: [&str; 16] = [
        "--bg",
        "x y",
        "",
        "say \"hi\"",
        "\"",
        "100%",
        "%PATH%",
        "%%",
        "a^b",
        "a&b|c",
        "<in> out",
        "!x!",
        "tail\\",
        "C:\\dir with space\\",
        "\\\"q\\\"",
        "ünï ✓",
    ];

    pub fn run() {
        let tests: [(&str, fn()); 7] = [
            (
                "an_exe_claude_gets_every_argument_unchanged",
                an_exe_claude_gets_every_argument_unchanged,
            ),
            (
                "a_cmd_claude_gets_every_argument_unchanged",
                a_cmd_claude_gets_every_argument_unchanged,
            ),
            (
                "the_exit_code_is_claudes_whole",
                the_exit_code_is_claudes_whole,
            ),
            (
                "the_launcher_runs_from_cmd_and_powershell_under_any_policy",
                the_launcher_runs_from_cmd_and_powershell_under_any_policy,
            ),
            (
                "the_launchers_own_folder_is_never_searched_for_claude",
                the_launchers_own_folder_is_never_searched_for_claude,
            ),
            (
                "a_missing_sidecar_or_claude_is_reported",
                a_missing_sidecar_or_claude_is_reported,
            ),
            (
                "a_line_break_for_a_cmd_claude_is_refused_not_altered",
                a_line_break_for_a_cmd_claude_is_refused_not_altered,
            ),
        ];
        let mut failed = Vec::new();
        for (name, test) in tests {
            match panic::catch_unwind(test) {
                Ok(()) => println!("test {name} ... ok"),
                Err(_) => {
                    println!("test {name} ... FAILED");
                    failed.push(name);
                }
            }
        }
        if !failed.is_empty() {
            println!("failed: {failed:?}");
            process::exit(101);
        }
        println!("test result: ok. {} passed", tests.len());
    }

    /// A launcher `claude-b.exe` for the instance `<root>/it's 100% b & c`, and a stand-in
    /// `claude.exe` in `fake`.
    struct Setup {
        root: tempfile::TempDir,
        bin: PathBuf,
        fake: PathBuf,
        instance: PathBuf,
    }

    impl Setup {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            let fake = root.path().join("fake bin");
            let instance = root.path().join("it's 100% b & c");
            for dir in [&bin, &fake, &instance] {
                fs::create_dir(dir).unwrap();
            }
            fs::copy(
                env!("CARGO_BIN_EXE_mccboard-shim"),
                bin.join("claude-b.exe"),
            )
            .unwrap();
            fs::write(bin.join("claude-b.shim"), sidecar::render(&instance)).unwrap();
            fs::copy(std::env::current_exe().unwrap(), fake.join("claude.exe")).unwrap();
            Self {
                root,
                bin,
                fake,
                instance,
            }
        }

        fn launcher(&self) -> PathBuf {
            self.bin.join("claude-b.exe")
        }

        /// A PATH with `dirs` first, then the system's (for cmd.exe and PowerShell).
        fn path(&self, dirs: &[&Path]) -> OsString {
            let system = std::env::var_os("PATH").unwrap_or_default();
            let mut all: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
            all.extend(std::env::split_paths(&system));
            std::env::join_paths(all).unwrap()
        }

        /// Runs `program` with `args` and the PATH holding `dirs` first.
        fn run(&self, program: &Path, args: &[&str], dirs: &[&Path]) -> Output {
            Command::new(program)
                .args(args)
                .env("PATH", self.path(dirs))
                .env_remove("CLAUDE_CONFIG_DIR")
                .env_remove("STAND_IN_EXIT")
                .current_dir(self.root.path())
                .output()
                .unwrap()
        }

        /// What the stand-in prints when started in the instance with `args`.
        fn expected(&self, args: &[&str]) -> Vec<String> {
            let mut lines = vec![format!(
                "dir={:?}",
                Some(OsString::from(self.instance.as_os_str()))
            )];
            lines.extend(
                args.iter()
                    .map(|arg| format!("arg={:?}", OsString::from(arg))),
            );
            lines
        }
    }

    fn lines(out: &Output) -> Vec<String> {
        assert!(
            out.status.code().is_some(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn an_exe_claude_gets_every_argument_unchanged() {
        let s = Setup::new();
        let out = s.run(&s.launcher(), &ARGS, &[&s.bin, &s.fake]);
        assert_eq!(
            lines(&out),
            s.expected(&ARGS),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.code(), Some(0));
    }

    /// `claude.cmd`, as an npm install has it: a batch file starting the real program with
    /// `%*`.
    fn a_cmd_claude_gets_every_argument_unchanged() {
        let s = Setup::new();
        // The real program sits outside the PATH; only the batch file knows it.
        let real = s.root.path().join("node_modules");
        let npm = s.root.path().join("npm & co");
        for dir in [&real, &npm] {
            fs::create_dir(dir).unwrap();
        }
        fs::copy(std::env::current_exe().unwrap(), real.join("claude.exe")).unwrap();
        fs::write(
            npm.join("claude.cmd"),
            format!(
                "@ECHO off\r\nSETLOCAL\r\n\"{}\" %*\r\n",
                real.join("claude.exe").display()
            ),
        )
        .unwrap();

        let out = s.run(&s.launcher(), &ARGS, &[&s.bin, &npm]);
        assert_eq!(
            lines(&out),
            s.expected(&ARGS),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let out = Command::new(s.launcher())
            .arg("--version")
            .env("PATH", s.path(&[&npm]))
            .env("STAND_IN_EXIT", "7")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(7));
    }

    fn the_exit_code_is_claudes_whole() {
        let s = Setup::new();
        // 0xC000013A, as a program ended by Ctrl+C exits: more than a byte.
        for code in ["0", "1", "7", "255", "256", "-1073741510"] {
            let out = Command::new(s.launcher())
                .env("PATH", s.path(&[&s.fake]))
                .env("STAND_IN_EXIT", code)
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(code.parse().unwrap()), "{code}");
        }
    }

    /// The launcher is a program, so script execution policies don't apply to it, and
    /// shells find it by `PATHEXT`.
    fn the_launcher_runs_from_cmd_and_powershell_under_any_policy() {
        let s = Setup::new();
        let dirs: [&Path; 2] = [&s.bin, &s.fake];
        let expected = s.expected(&["--bg", "x y"]);

        // cmd.exe has its own quoting: its command line goes as written.
        let out = Command::new("cmd")
            .raw_arg("/d /c claude-b --bg \"x y\"")
            .env("PATH", s.path(&dirs))
            .env_remove("CLAUDE_CONFIG_DIR")
            .current_dir(s.root.path())
            .output()
            .unwrap();
        assert_eq!(
            lines(&out),
            expected,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let mut shells = vec!["powershell"];
        if Command::new("pwsh").arg("-Version").output().is_ok() {
            shells.push("pwsh");
        }
        for shell in shells {
            for policy in ["Restricted", "RemoteSigned", "Bypass"] {
                let out = s.run(
                    Path::new(shell),
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        policy,
                        "-Command",
                        "claude-b --bg 'x y'; exit $LASTEXITCODE",
                    ],
                    &dirs,
                );
                assert_eq!(
                    lines(&out),
                    expected,
                    "{shell} {policy}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(out.status.code(), Some(0), "{shell} {policy}");
            }
        }
    }

    fn the_launchers_own_folder_is_never_searched_for_claude() {
        let s = Setup::new();
        // A `claude.exe` in the launchers' folder: another launcher, here with no sidecar.
        fs::copy(
            env!("CARGO_BIN_EXE_mccboard-shim"),
            s.bin.join("claude.exe"),
        )
        .unwrap();

        let out = s.run(&s.launcher(), &["agents"], &[&s.bin, &s.fake]);

        assert_eq!(
            lines(&out),
            s.expected(&["agents"]),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn a_missing_sidecar_or_claude_is_reported() {
        let s = Setup::new();
        // No system PATH: a Claude Code installed on the machine must not be found.
        let out = Command::new(s.launcher())
            .env("PATH", &s.bin)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("claude-b: cannot find Claude Code"),
            "{stderr}"
        );
        assert!(out.stdout.is_empty());

        fs::remove_file(s.bin.join("claude-b.shim")).unwrap();
        let out = s.run(&s.launcher(), &[], &[&s.bin, &s.fake]);
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("claude-b.shim"), "{stderr}");
        assert!(stderr.contains("mccboard launcher sync"), "{stderr}");
        assert!(out.stdout.is_empty());
    }

    fn a_line_break_for_a_cmd_claude_is_refused_not_altered() {
        let s = Setup::new();
        let npm = s.root.path().join("npm");
        fs::create_dir(&npm).unwrap();
        fs::write(npm.join("claude.cmd"), "@echo ran %*\r\n").unwrap();

        let out = s.run(&s.launcher(), &["a\nb"], &[&s.bin, &npm]);

        assert_eq!(out.status.code(), Some(1));
        assert!(
            out.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("batch file"), "{stderr}");
    }
}
