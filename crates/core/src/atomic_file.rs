//! Writing mccboard's own files whole: a temporary file in the same folder, then a rename, so
//! a reader (another front end, Explorer, the shell) never sees half a file and an interrupted
//! write leaves the old one.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tells apart the temporary files of writes running at once in this process.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path` whole, creating its folder first. The temporary file,
/// `.<name>.<pid>-<n>.tmp` next to it, never stays behind when the write or the rename fails.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}-{n}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .and_then(|()| fs::rename(&temporary, path))
        .inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_a_new_file_in_a_new_folder_and_leaves_nothing_else() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("state");
        let path = dir.join("record.json");

        write(&path, b"first").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(names(&dir), ["record.json"]);
    }

    #[test]
    fn replaces_an_existing_file_whole() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("icon.ico");
        fs::write(&path, b"old and longer").unwrap();

        write(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(names(home.path()), ["icon.ico"]);
    }

    #[test]
    fn a_failed_rename_keeps_the_old_entry_and_no_temporary_file() {
        let home = tempfile::tempdir().unwrap();
        // A non-empty folder can't be replaced by a file.
        let path = home.path().join("taken");
        fs::create_dir_all(path.join("inside")).unwrap();

        assert!(write(&path, b"text").is_err());

        assert!(path.join("inside").is_dir());
        assert_eq!(names(home.path()), ["taken"]);
    }
}
