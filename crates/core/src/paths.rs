use std::path::{Path, PathBuf};

/// Where mccboard keeps its own files: `~/.mccboard/`, never inside `~/.claude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            root: home.join(".mccboard"),
        }
    }

    /// `~/.mccboard/`
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `~/.mccboard/config.toml`: hand-editable settings and registrations.
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// `~/.mccboard/state/`: data mccboard writes on its own (readings, backoff, backups).
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
}

/// Turns a path the user typed into the form mccboard stores: `~` expanded to the home,
/// required to be absolute, `.` and `..` resolved lexically, trailing separators dropped and,
/// on Windows, the `\\?\` prefix removed. Links are not followed: an instance is the folder
/// the user named.
pub(crate) fn resolve(home: &Path, path: &Path) -> Result<PathBuf, crate::Error> {
    let expanded = match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    };
    if !expanded.is_absolute() {
        return Err(crate::Error::RelativePath {
            path: path.to_path_buf(),
        });
    }
    Ok(lexical(&expanded))
}

/// `path` with `.` and `..` resolved lexically and, on Windows, the `\\?\` prefix removed.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    lexical(path)
}

fn lexical(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::VerbatimDisk(letter) => out.push(format!("{}:", char::from(letter))),
                Prefix::VerbatimUNC(server, share) => {
                    let mut unc = std::ffi::OsString::from(r"\\");
                    unc.push(server);
                    unc.push(r"\");
                    unc.push(share);
                    out.push(unc);
                }
                _ => out.push(component.as_os_str()),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A key under which two spellings of the same folder compare equal: lexically normalised
/// and, on Windows, where paths are case-insensitive, lowercased.
pub(crate) fn key(path: &Path) -> String {
    let normal = lexical(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        normal.to_lowercase()
    } else {
        normal
    }
}

pub(crate) fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}
