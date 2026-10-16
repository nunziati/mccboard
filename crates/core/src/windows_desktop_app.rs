//! The Claude desktop app's MSIX packages, from Windows' packaging API (`kernel32`'s appmodel
//! functions): the packages of a family installed for the current user, and their install
//! folders. No PowerShell, no WinRT.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
};
use windows_sys::Win32::Storage::Packaging::Appx::{
    GetPackagePathByFullName, GetPackagesByPackageFamily,
};

use crate::desktop_launch::DesktopPackage;

/// Every package of `families` installed for the current user, with its install folder. A
/// package whose folder can't be read (being removed by an update) is left out.
pub(crate) fn packages(families: &[String]) -> io::Result<Vec<DesktopPackage>> {
    let mut out = Vec::new();
    for family in families {
        for full_name in full_names(family)? {
            if let Ok(install_dir) = install_dir(&full_name) {
                out.push(DesktopPackage {
                    full_name,
                    install_dir,
                });
            }
        }
    }
    Ok(out)
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

/// The package full names of `family` (`Claude_1.0.2342.0_x64__pzs8sxrjxfjjc`); empty when
/// none is installed.
fn full_names(family: &str) -> io::Result<Vec<String>> {
    let family = wide(family);
    loop {
        let (mut count, mut len) = (0u32, 0u32);
        // SAFETY: the sizing call: null buffers with zero sizes, as the API documents.
        let rc = unsafe {
            GetPackagesByPackageFamily(
                family.as_ptr(),
                &mut count,
                null_mut(),
                &mut len,
                null_mut(),
            )
        };
        if (rc == ERROR_SUCCESS && count == 0) || rc == APPMODEL_ERROR_NO_PACKAGE {
            return Ok(Vec::new());
        }
        if rc != ERROR_INSUFFICIENT_BUFFER {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let mut names = vec![null_mut::<u16>(); count as usize];
        let mut buffer = vec![0u16; len as usize];
        // SAFETY: both arrays are as large as the sizing call asked; the API fills `names`
        // with pointers into `buffer`.
        let rc = unsafe {
            GetPackagesByPackageFamily(
                family.as_ptr(),
                &mut count,
                names.as_mut_ptr(),
                &mut len,
                buffer.as_mut_ptr(),
            )
        };
        if rc == ERROR_INSUFFICIENT_BUFFER {
            continue; // a package was added in between (an update): ask again
        }
        if rc == APPMODEL_ERROR_NO_PACKAGE {
            return Ok(Vec::new());
        }
        if rc != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        // Each name points into `buffer`: read it from there, by offset, up to its NUL.
        let base = buffer.as_ptr() as usize;
        let count = (count as usize).min(names.len());
        return Ok(names[..count]
            .iter()
            .filter_map(|&name| {
                let start = (name as usize).checked_sub(base)? / 2;
                let rest = buffer.get(start..)?;
                let end = rest.iter().position(|&c| c == 0)?;
                Some(String::from_utf16_lossy(&rest[..end]))
            })
            .collect());
    }
}

/// The install folder of the package `full_name` (`C:\Program Files\WindowsApps\Claude_…`).
fn install_dir(full_name: &str) -> io::Result<PathBuf> {
    let name = wide(full_name);
    let mut len = 0u32;
    // SAFETY: the sizing call, with a null buffer of length zero.
    let rc = unsafe { GetPackagePathByFullName(name.as_ptr(), &mut len, null_mut()) };
    if rc != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: `buffer` holds `len` characters, as the sizing call asked.
    let rc = unsafe { GetPackagePathByFullName(name.as_ptr(), &mut len, buffer.as_mut_ptr()) };
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    if let Some(nul) = buffer.iter().position(|&c| c == 0) {
        buffer.truncate(nul);
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_family_has_no_package() {
        assert_eq!(
            packages(&["Mccboard.NoSuchPackage_0000000000000".to_owned()]).unwrap(),
            Vec::new()
        );
    }
}
