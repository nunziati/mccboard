//! Folders mccboard never makes an instance in, nor deletes: one rule for adding and removing.
//!
//! A folder is protected when it holds the home, the primary instance or mccboard's own files,
//! sits inside the primary instance or mccboard's files, or is itself a link (a symlink or a
//! junction, whatever it points to). Paths are compared as spelled and again once links are
//! resolved, so no link or `..` spelling gets around the rule.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{links, paths};

/// Whether `dir` is a protected folder in `home`, where mccboard keeps its files in `mccboard`
/// (`~/.mccboard`).
pub(crate) fn is_protected(home: &Path, mccboard: &Path, dir: &Path) -> bool {
    let primary = home.join(".claude");
    links::is_link(dir)
        || [home, primary.as_path(), mccboard]
            .into_iter()
            .any(|protected| holds(dir, protected))
        || holds(&primary, dir)
        || holds(mccboard, dir)
}

/// Whether `inner` is `outer` or somewhere inside it, by spelling or once links are resolved.
pub(crate) fn holds(outer: &Path, inner: &Path) -> bool {
    let inside = |outer: &Path, inner: &Path| {
        Path::new(&paths::key(inner)).starts_with(Path::new(&paths::key(outer)))
    };
    if inside(outer, inner) {
        return true;
    }
    match (resolved(outer), resolved(inner)) {
        (Some(outer), Some(inner)) => inside(&outer, &inner),
        _ => false,
    }
}

/// `path` with every link resolved. A path that doesn't exist (yet) is resolved through its
/// nearest existing folder, so a folder about to be created behind a link is still placed.
fn resolved(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = fs::canonicalize(current) {
            let mut out = paths::normalize(&real);
            out.extend(missing.iter().rev());
            return Some(out);
        }
        missing.push(current.file_name()?);
        current = current.parent()?;
    }
}
