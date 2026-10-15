//! The user PATH on Windows: the `Path` value under `HKCU\Environment`.
//!
//! The value is usually `REG_EXPAND_SZ` (it holds `%USERPROFILE%` and the like), so it is read
//! and written raw, keeping its type: writing it as a plain string would stop the expansion.
//! After a change, running programs (Explorer first) are told the environment changed, so
//! terminals opened from then on get the new PATH. `setx` is avoided: it truncates at 1024
//! characters.

use std::borrow::Cow;
use std::io;
use std::path::Path;

use winreg::RegKey;
use winreg::RegValue;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ, RegType};

const KEY: &str = "Environment";
const VALUE: &str = "Path";

pub(crate) fn contains(dir: &Path) -> io::Result<bool> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_READ) {
        Ok(key) => key,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(read(&key)?.is_some_and(|(path, _)| has_entry(&path, dir)))
}

pub(crate) fn add(dir: &Path) -> io::Result<()> {
    let (key, _) =
        RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(KEY, KEY_READ | KEY_WRITE)?;
    let (path, vtype) = read(&key)?.unwrap_or((String::new(), REG_EXPAND_SZ));
    if has_entry(&path, dir) {
        return Ok(());
    }
    let dir = dir.to_string_lossy();
    let trimmed = path.trim_end_matches(';');
    let new_path = if trimmed.is_empty() {
        dir.into_owned()
    } else {
        format!("{trimmed};{dir}")
    };
    let bytes: Vec<u8> = new_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    key.set_raw_value(
        VALUE,
        &RegValue {
            bytes: Cow::Owned(bytes),
            vtype,
        },
    )?;
    broadcast_environment_change();
    Ok(())
}

/// The PATH text and its type; `None` when the value doesn't exist. A value of another type
/// than a string is treated as missing, and rewritten as `REG_EXPAND_SZ`.
fn read(key: &RegKey) -> io::Result<Option<(String, RegType)>> {
    let value = match key.get_raw_value(VALUE) {
        Ok(value) => value,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if value.vtype != REG_SZ && value.vtype != REG_EXPAND_SZ {
        return Ok(None);
    }
    let units: Vec<u16> = value
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Ok(Some((String::from_utf16_lossy(&units[..end]), value.vtype)))
}

/// Whether one of the `;`-separated entries is `dir`, after expanding `%VARIABLES%`, ignoring
/// case and trailing separators.
fn has_entry(path: &str, dir: &Path) -> bool {
    path.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| crate::paths::same(Path::new(&expand(entry)), dir))
}

/// Expands `%NAME%` from the process environment; unknown names stay as written.
fn expand(entry: &str) -> String {
    let mut out = String::new();
    let mut rest = entry;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tells top-level windows that the environment changed (`WM_SETTINGCHANGE` with
/// `"Environment"`), waiting at most 5 s for hung ones.
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    let parameter: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut result = 0;
    // SAFETY: `parameter` is a NUL-terminated UTF-16 string that outlives the call, and
    // `result` is a valid place for the answer.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            parameter.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}
