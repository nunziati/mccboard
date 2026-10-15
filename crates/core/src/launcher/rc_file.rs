//! Linux launchers: alias lines in a block of the user's shell rc file marked as mccboard's.
//!
//! ```sh
//! # >>> mccboard launchers >>>
//! # Managed by mccboard: changes inside this block are overwritten. …
//! alias claude-b='CLAUDE_CONFIG_DIR=/home/me/.claude-b claude'
//! # <<< mccboard launchers <<<
//! ```
//!
//! Only the block is ever rewritten; everything around it stays byte for byte. An rc file that
//! is a link (dotfiles managers) is edited where it points. A block without its end marker is
//! reported and never rewritten.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{Found, Wanted, shell};
use crate::Error;

const BEGIN: &str = "# >>> mccboard launchers >>>";
const END: &str = "# <<< mccboard launchers <<<";
const NOTE: &str = "# Managed by mccboard: changes inside this block are overwritten. \
                    Rename launchers with `mccboard account set`.";

pub(super) struct RcFile {
    path: PathBuf,
}

/// Where the block sits in the file: line indices of the markers, inclusive.
struct Block {
    begin: usize,
    end: usize,
}

impl RcFile {
    /// The rc file of the user's login shell: `~/.zshrc` for zsh, `~/.bashrc` otherwise.
    pub fn of(home: &Path, login_shell: Option<PathBuf>) -> Self {
        let zsh = login_shell
            .as_deref()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "zsh");
        Self {
            path: home.join(if zsh { ".zshrc" } else { ".bashrc" }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The aliases inside the block.
    pub fn read(&self) -> Result<Vec<Found>, Error> {
        let text = self.text()?;
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let Some(block) = self.block(&lines)? else {
            return Ok(Vec::new());
        };
        Ok(lines[block.begin + 1..block.end]
            .iter()
            .filter_map(|line| parse_alias(line.trim_end()))
            .collect())
    }

    /// Makes the block hold exactly the `wanted` aliases; with none, removes the block. Writes
    /// only when something changes.
    pub fn write(&self, wanted: &[Wanted]) -> Result<(), Error> {
        let text = self.text()?;
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let block = self.block(&lines)?;

        let new_block = if wanted.is_empty() {
            String::new()
        } else {
            let mut aliases: Vec<String> = wanted
                .iter()
                .map(|w| alias_line(&w.name, &w.config_dir))
                .collect();
            aliases.sort();
            let mut block = format!("{BEGIN}\n{NOTE}\n");
            for alias in aliases {
                block.push_str(&alias);
                block.push('\n');
            }
            block.push_str(END);
            block.push('\n');
            block
        };

        let new_text = match block {
            Some(Block { begin, end }) => {
                let mut before = lines[..begin].concat();
                let after = lines[end + 1..].concat();
                // The blank line put before the block goes with it.
                if new_block.is_empty() && (before.ends_with("\n\n") || before == "\n") {
                    before.pop();
                }
                before + &new_block + &after
            }
            None if new_block.is_empty() => return Ok(()),
            None => {
                let mut new_text = text.clone();
                if !new_text.is_empty() {
                    if !new_text.ends_with('\n') {
                        new_text.push('\n');
                    }
                    new_text.push('\n');
                }
                new_text + &new_block
            }
        };
        if new_text != text {
            self.save(&new_text)?;
        }
        Ok(())
    }

    fn text(&self) -> Result<String, Error> {
        match fs::read(&self.path) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| Error::LauncherUnreadable {
                path: self.path.clone(),
                source: io::Error::new(io::ErrorKind::InvalidData, "not UTF-8 text"),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(source) => Err(Error::LauncherUnreadable {
                path: self.path.clone(),
                source,
            }),
        }
    }

    fn block(&self, lines: &[&str]) -> Result<Option<Block>, Error> {
        let corrupt = || Error::LauncherBlockCorrupt {
            path: self.path.clone(),
        };
        let mut begin = None;
        let mut block = None;
        for (index, line) in lines.iter().enumerate() {
            let line = line.trim_end();
            if line == BEGIN {
                if begin.is_some() || block.is_some() {
                    return Err(corrupt());
                }
                begin = Some(index);
            } else if line == END {
                match begin.take() {
                    Some(begin) if block.is_none() => block = Some(Block { begin, end: index }),
                    _ => return Err(corrupt()),
                }
            }
        }
        if begin.is_some() {
            return Err(corrupt());
        }
        Ok(block)
    }

    /// Replaces the file atomically (a temporary file next to it, then a rename), keeping its
    /// permissions. A link is followed, so the link stays a link.
    fn save(&self, text: &str) -> Result<(), Error> {
        let unwritable = |source| Error::LauncherUnwritable {
            path: self.path.clone(),
            source,
        };
        let target = match fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.is_symlink() => fs::canonicalize(&self.path).map_err(unwritable)?,
            _ => self.path.clone(),
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
        result.map_err(unwritable)
    }
}

/// The alias mccboard writes: `alias claude-b='CLAUDE_CONFIG_DIR=/home/me/.claude-b claude'`.
/// An alias passes every argument through to `claude`.
pub(super) fn alias_line(name: &str, config_dir: &Path) -> String {
    let dir = shell::quote_if_needed(&config_dir.to_string_lossy());
    format!(
        "alias {name}={}",
        shell::quote(&format!("CLAUDE_CONFIG_DIR={dir} claude"))
    )
}

/// An `alias name=value` line, with the config directory its value sets, if it sets one.
fn parse_alias(line: &str) -> Option<Found> {
    let words = shell::words(line)?;
    let [keyword, definition, ..] = words.as_slice() else {
        return None;
    };
    if keyword != "alias" {
        return None;
    }
    let (name, value) = definition.split_once('=')?;
    let config_dir = shell::words(value)
        .and_then(|words| words.into_iter().next())
        .and_then(|word| word.strip_prefix("CLAUDE_CONFIG_DIR=").map(PathBuf::from));
    Some(Found {
        name: name.to_owned(),
        config_dir,
        content: line.to_owned(),
        managed: true,
    })
}
