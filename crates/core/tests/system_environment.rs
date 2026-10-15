//! The real environment port on this machine: Claude Code processes are found with the config
//! directory they use, so the session guard sees real sessions.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Child, Command};

use mccboard_core::{Core, Environment, ProcessConfigDir, SystemEnvironment};

/// A stand-in Claude Code process: `sleep` copied to a file named `claude`.
struct FakeClaude(Child);

impl FakeClaude {
    fn start(bin: &Path, home: &Path, config_dir: Option<&Path>) -> Self {
        let mut command = Command::new(bin);
        command
            .arg("30")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("HOME", home);
        if let Some(dir) = config_dir {
            command.env("CLAUDE_CONFIG_DIR", dir);
        }
        let child = command.spawn().unwrap();
        // Let it exec before it is listed.
        for _ in 0..50 {
            let exe = fs::read_link(format!("/proc/{}/exe", child.id()));
            if exe.is_ok_and(|exe| exe == bin) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Self(child)
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn claude_code_processes_are_listed_with_their_config_directory() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin").join("claude");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::copy("/bin/sleep", &bin).unwrap();
    let b = home.path().join(".claude-b");
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(home.path().join(".claude")).unwrap();

    let linked = FakeClaude::start(&bin, home.path(), Some(&b));
    let default = FakeClaude::start(&bin, home.path(), None);
    // Its own home, elsewhere: its default folder is not this home's primary instance.
    let other_home = tempfile::tempdir().unwrap();
    let elsewhere = FakeClaude::start(&bin, other_home.path(), None);

    let processes = SystemEnvironment.claude_processes();
    let of = |pid: u32| processes.iter().find(|p| p.pid == pid);
    assert_eq!(
        of(linked.0.id()).map(|p| &p.config_dir),
        Some(&ProcessConfigDir::Dir { path: b.clone() }),
        "{processes:?}"
    );
    assert_eq!(
        of(default.0.id()).map(|p| &p.config_dir),
        Some(&ProcessConfigDir::Dir {
            path: home.path().join(".claude")
        })
    );
    assert_eq!(of(linked.0.id()).unwrap().name, "claude");
    // mccboard itself is never listed.
    assert!(of(std::process::id()).is_none());

    let core = Core::new(home.path(), SystemEnvironment);
    let using: Vec<u32> = core
        .processes_using(&b)
        .unwrap()
        .iter()
        .map(|p| p.pid)
        .collect();
    assert_eq!(using, vec![linked.0.id()]);
    let primary: Vec<u32> = core
        .processes_using(&home.path().join(".claude"))
        .unwrap()
        .iter()
        .map(|p| p.pid)
        .collect();
    assert_eq!(primary, vec![default.0.id()]);
    assert!(of(elsewhere.0.id()).is_some());
}

#[test]
fn file_links_are_always_allowed_off_windows() {
    assert!(SystemEnvironment.file_links_allowed());
}
