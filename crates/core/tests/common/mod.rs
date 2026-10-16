//! Temporary homes laid out like a real one, for tests through the core's public interface.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiff::Timestamp;
use mccboard_core::Core;
use mccboard_core::testing::FakeEnvironment;
use serde_json::json;
use tempfile::TempDir;

/// A token-shaped fixture value. It must never appear in anything the core produces.
pub const FIXTURE_TOKEN: &str = "sk-ant-oat01-FIXTURE-TOKEN-must-never-leak-0123456789";
pub const FIXTURE_REFRESH: &str = "sk-ant-ort01-FIXTURE-REFRESH-must-never-leak-9876543210";

pub const NOW: &str = "2026-10-15T12:00:00Z";

pub fn now() -> Timestamp {
    NOW.parse().unwrap()
}

pub struct TempHome {
    dir: TempDir,
}

impl TempHome {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// A core over this home, with a fake environment whose clock reads [`NOW`] and whose
    /// Start menu is [`TempHome::start_menu`] (used when the fake plays Windows).
    pub fn core(&self) -> (Core, FakeEnvironment) {
        let env = FakeEnvironment::new(now());
        env.set_start_menu(Some(&self.start_menu()));
        (Core::new(self.path(), env.clone()), env)
    }

    /// The Start menu's Programs folder of the fake Windows user, under this home (not
    /// created: the first shortcut creates it).
    pub fn start_menu(&self) -> PathBuf {
        self.path()
            .join("AppData")
            .join("Roaming")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    }

    /// Creates `~/.claude`, the primary instance's config directory.
    pub fn primary(&self) -> Instance {
        let dir = self.path().join(".claude");
        fs::create_dir_all(&dir).unwrap();
        Instance {
            global_config: self.path().join(".claude.json"),
            dir,
        }
    }

    /// Creates `~/<name>`, a linked instance's config directory.
    pub fn linked(&self, name: &str) -> Instance {
        let dir = self.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        Instance {
            global_config: dir.join(".claude.json"),
            dir,
        }
    }

    pub fn write(&self, relative: &str, contents: &str) {
        let path = self.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

pub struct Instance {
    pub dir: PathBuf,
    pub global_config: PathBuf,
}

impl Instance {
    /// Writes the instance's `.claude.json` with an `oauthAccount`.
    pub fn logged_in_as(&self, uuid: &str, email: &str) -> &Self {
        self.global_config(json!({
            "numStartups": 3,
            "oauthAccount": {
                "accountUuid": uuid,
                "emailAddress": email,
                "organizationUuid": "00000000-0000-4000-8000-0000000000aa",
            },
            "autoConnectIde": true,
        }))
    }

    pub fn global_config(&self, value: serde_json::Value) -> &Self {
        fs::write(&self.global_config, value.to_string()).unwrap();
        self
    }

    /// Writes `.credentials.json` shaped like Claude Code's, with dummy token values.
    pub fn credentials_expiring(&self, expires_at: Timestamp) -> &Self {
        self.raw_credentials(
            &json!({
                "claudeAiOauth": {
                    "accessToken": FIXTURE_TOKEN,
                    "refreshToken": FIXTURE_REFRESH,
                    "expiresAt": expires_at.as_millisecond(),
                    "scopes": ["user:inference", "user:profile"],
                    "subscriptionType": "max",
                }
            })
            .to_string(),
        )
    }

    pub fn raw_credentials(&self, contents: &str) -> &Self {
        fs::write(self.dir.join(".credentials.json"), contents).unwrap();
        self
    }
}

pub const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
pub const UUID_B: &str = "22222222-2222-4222-8222-222222222222";
pub const UUID_C: &str = "33333333-3333-4333-8333-333333333333";

impl TempHome {
    /// `~/.mccboard/config.toml`.
    pub fn config_path(&self) -> PathBuf {
        self.path().join(".mccboard").join("config.toml")
    }

    /// The text of `~/.mccboard/config.toml`, or `None` when it doesn't exist.
    pub fn config_text(&self) -> Option<String> {
        fs::read_to_string(self.config_path()).ok()
    }

    /// Writes `~/.mccboard/config.toml` as a user editing it by hand would.
    pub fn write_config(&self, contents: &str) {
        fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        fs::write(self.config_path(), contents).unwrap();
    }
}

impl Instance {
    /// Writes `.credentials.json` with a given dummy access token, so tests can tell which
    /// instance's token a call used.
    pub fn credentials_with_token(&self, token: &str, expires_at: Timestamp) -> &Self {
        self.raw_credentials(
            &json!({
                "claudeAiOauth": {
                    "accessToken": token,
                    "refreshToken": FIXTURE_REFRESH,
                    "expiresAt": expires_at.as_millisecond(),
                    "scopes": ["user:inference", "user:profile"],
                }
            })
            .to_string(),
        )
    }

    /// The bytes of `.credentials.json`, to check mccboard never wrote it.
    pub fn credentials_bytes(&self) -> Vec<u8> {
        fs::read(self.dir.join(".credentials.json")).unwrap()
    }
}

impl TempHome {
    /// Every file under `~/.mccboard/` (config and state), as text.
    pub fn mccboard_files(&self) -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
                    out.push((path, text));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.path().join(".mccboard"), &mut out);
        out
    }
}

/// The packaged desktop app's program, as Windows lists it.
pub const DESKTOP_EXE: &str =
    r"C:\Program Files\WindowsApps\Claude_1.0.2342.0_x64__pzs8sxrjxfjjc\app\Claude.exe";

/// A process of the packaged desktop app with command line `args` after the program.
pub fn desktop_process(pid: u32, args: &[&str]) -> mccboard_core::ProcessListing {
    mccboard_core::ProcessListing {
        pid,
        parent_pid: Some(1),
        name: "Claude.exe".to_owned(),
        exe: Some(PathBuf::from(DESKTOP_EXE)),
        args: std::iter::once(DESKTOP_EXE)
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    }
}

/// `--user-data-dir=<folder>`, as mccboard passes it.
pub fn user_data_dir(folder: &Path) -> String {
    format!("--user-data-dir={}", folder.display())
}

/// Sets the modification time of the folder `dir`. On Windows a folder opens only with
/// `FILE_FLAG_BACKUP_SEMANTICS`, and setting its times needs `FILE_WRITE_ATTRIBUTES`.
pub fn set_folder_modified(dir: &Path, modified: SystemTime) {
    #[cfg(windows)]
    let folder = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
    };
    #[cfg(not(windows))]
    let folder = fs::File::open(dir);
    folder.unwrap().set_modified(modified).unwrap();
}
