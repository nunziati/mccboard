//! The alias block that mccboard up to 0.1 wrote in the user's shell rc file for the Linux
//! launchers, which are now scripts ([`super::script_folder`]). A sync removes the block from
//! `~/.bashrc` and `~/.zshrc`:
//!
//! ```sh
//! # >>> mccboard launchers >>>
//! # Managed by mccboard: changes inside this block are overwritten. …
//! alias claude-b='CLAUDE_CONFIG_DIR=/home/me/.claude-b claude'
//! # <<< mccboard launchers <<<
//! ```
//!
//! Only the block goes: everything around it stays byte for byte. An rc file that is a link
//! (dotfiles managers) is edited where it points. A block without its end marker stays as it is.

use std::fs;
use std::path::{Path, PathBuf};

const BEGIN: &str = "# >>> mccboard launchers >>>";
const END: &str = "# <<< mccboard launchers <<<";

/// The rc files that may hold the block: those of bash and zsh.
pub(super) fn candidates(home: &Path) -> [PathBuf; 2] {
    [home.join(".bashrc"), home.join(".zshrc")]
}

/// Removes the block from the rc file at `path`, when it holds a complete one. Leaves the file
/// alone otherwise, and when it cannot be read or written: the scripts work without this.
pub(super) fn remove_block(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some((begin, end)) = block(&lines) else {
        return;
    };
    let mut before = lines[..begin].concat();
    let after = lines[end + 1..].concat();
    // The blank line put before the block goes with it.
    if before.ends_with("\n\n") || before == "\n" {
        before.pop();
    }
    let _ = save(path, &(before + &after));
}

/// The line indices of the markers of the one complete block, inclusive.
fn block(lines: &[&str]) -> Option<(usize, usize)> {
    let mut begin = None;
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim_end();
        if line == BEGIN {
            if begin.is_some() || found.is_some() {
                return None;
            }
            begin = Some(index);
        } else if line == END {
            match begin.take() {
                Some(begin) if found.is_none() => found = Some((begin, index)),
                _ => return None,
            }
        }
    }
    if begin.is_some() { None } else { found }
}

/// Replaces the file atomically (a temporary file next to it, then a rename), keeping its
/// permissions. A link is followed and stays a link.
fn save(path: &Path, text: &str) -> std::io::Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = target.parent().unwrap_or(Path::new("."));
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = dir.join(format!(".{file_name}.mccboard-{}.tmp", std::process::id()));
    let result = fs::write(&temporary, text)
        .and_then(|()| match fs::metadata(&target) {
            Ok(meta) => fs::set_permissions(&temporary, meta.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| fs::rename(&temporary, &target));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
