//! Reading links: symlinks on every OS and junctions on Windows, told apart from real files
//! and folders without following them.
//!
//! Rust's standard library reports a Windows junction as a symlink (`is_symlink()` is true
//! for every name-surrogate reparse point) and `read_link` reads both, so one code path
//! covers both kinds of link.

use std::fs;
use std::path::{Path, PathBuf};

use crate::paths;

/// What is at a path, without following a link there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Missing,
    /// A real file or folder.
    Real,
    /// A link (symlink or junction). `target` is its first hop, made absolute against the
    /// link's folder when the link is relative; `None` when it can't be read.
    Link {
        target: Option<PathBuf>,
    },
}

pub(crate) fn inspect(path: &Path) -> Entry {
    match fs::symlink_metadata(path) {
        Err(_) => Entry::Missing,
        Ok(meta) if meta.file_type().is_symlink() => Entry::Link {
            target: fs::read_link(path).ok().map(|target| {
                let parent = path.parent().unwrap_or(Path::new(""));
                paths::normalize(&parent.join(target))
            }),
        },
        Ok(_) => Entry::Real,
    }
}

pub(crate) fn is_link(path: &Path) -> bool {
    matches!(inspect(path), Entry::Link { .. })
}

/// The real item behind a path of the primary instance: the path itself, or, when the
/// primary's item is a link of its own, where its links finally lead. Links to it must point
/// there, in a single hop. Links are followed hop by hop, keeping their spelling (no
/// canonical form, which on Windows would expand short names).
pub(crate) fn real_item(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    // A bound on hops, so a link loop ends.
    for _ in 0..40 {
        match inspect(&current) {
            Entry::Link {
                target: Some(target),
            } => current = target,
            _ => return current,
        }
    }
    path.to_path_buf()
}

/// Where a path finally leads, every link followed; `None` if it leads nowhere.
pub(crate) fn destination(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().map(|p| paths::normalize(&p))
}

/// Whether two paths name the same place, the last component compared as is (never
/// followed) and the folders it sits in compared after resolving links and, on Windows, short
/// names. So `~/.claude/settings.json` reached through a linked home still matches, while a
/// link to a link doesn't.
pub(crate) fn same_place(a: &Path, b: &Path) -> bool {
    if paths::same(a, b) {
        return true;
    }
    let resolved = |p: &Path| -> Option<PathBuf> {
        let parent = fs::canonicalize(p.parent()?).ok()?;
        Some(paths::normalize(&parent.join(p.file_name()?)))
    };
    match (resolved(a), resolved(b)) {
        (Some(a), Some(b)) => paths::same(&a, &b),
        _ => false,
    }
}
