//! The Windows shell, for desktop profiles (ticket #37): Start-menu shortcuts with an
//! AppUserModelID, the Start menu's Programs folder, a program's icon, and the property store
//! of a window (used by `windows_taskbar`).
//!
//! The COM parts use the `windows` crate (windows-sys has no COM interfaces); the flat Win32
//! parts use windows-sys, as the rest of mccboard does.

use std::io;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
    PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{HSTRING, Interface, PWSTR};

use crate::Image;
use crate::desktop_identity::{Shortcut, WindowIdentity};

/// COM initialised on the current thread (single-threaded apartment) for as long as it lives.
/// A thread COM was already initialised on in another mode keeps it, and COM still works.
pub(crate) struct Com {
    uninitialize: bool,
}

impl Com {
    pub(crate) fn init() -> Com {
        // S_OK and S_FALSE both need a matching CoUninitialize; RPC_E_CHANGED_MODE doesn't.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Com {
            uninitialize: result.is_ok(),
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn io_error(error: windows::core::Error) -> io::Error {
    io::Error::other(error.message())
}

/// A `VT_LPWSTR` PROPVARIANT borrowing `wide` (NUL-terminated). `IPropertyStore::SetValue`
/// copies it. It is never cleared: clearing would free Rust's memory with CoTaskMemFree.
fn lpwstr(wide: &[u16]) -> PROPVARIANT {
    let mut value = PROPVARIANT::default();
    unsafe {
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = PWSTR(wide.as_ptr() as *mut u16);
    }
    value
}

fn set_string(store: &IPropertyStore, key: &PROPERTYKEY, text: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe { store.SetValue(key, &lpwstr(&wide)) }
}

/// The current user's Start-menu Programs folder.
pub(crate) fn start_menu_programs() -> io::Result<PathBuf> {
    unsafe {
        let path =
            SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).map_err(io_error)?;
        let folder = PathBuf::from(path.to_hstring().to_os_string());
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(folder)
    }
}

/// Writes `shortcut` as a shell link, replacing a file already at its path.
pub(crate) fn write_shortcut(shortcut: &Shortcut) -> io::Result<()> {
    let _com = Com::init();
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io_error)?;
        link.SetPath(&HSTRING::from(shortcut.target.as_path()))
            .map_err(io_error)?;
        link.SetArguments(&HSTRING::from(shortcut.arguments.as_str()))
            .map_err(io_error)?;
        if let Some(icon) = &shortcut.icon {
            link.SetIconLocation(&HSTRING::from(icon.as_path()), 0)
                .map_err(io_error)?;
        }
        link.SetDescription(&HSTRING::from(shortcut.description.as_str()))
            .map_err(io_error)?;
        let store: IPropertyStore = link.cast().map_err(io_error)?;
        set_string(&store, &PKEY_AppUserModel_ID, &shortcut.app_user_model_id).map_err(io_error)?;
        store.Commit().map_err(io_error)?;
        link.cast::<IPersistFile>()
            .map_err(io_error)?
            .Save(&HSTRING::from(shortcut.path.as_path()), true)
            .map_err(io_error)
    }
}

/// Tags the window `hwnd` (any process) with `identity` through its property store: the
/// relaunch command, display name and icon first, then the AppUserModelID, which moves the
/// window to the profile's taskbar button.
///
/// # Safety
/// COM must be initialised on the calling thread ([`Com`]). `hwnd` may be any window handle,
/// even one destroyed meanwhile: the call then fails.
pub(crate) unsafe fn tag_window(
    hwnd: *mut core::ffi::c_void,
    identity: &WindowIdentity,
) -> windows::core::Result<()> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(HWND(hwnd))? };
    // The relaunch properties count only together, and only on a window with an explicit
    // AppUserModelID.
    if let Some(command) = &identity.relaunch_command {
        set_string(&store, &PKEY_AppUserModel_RelaunchCommand, command)?;
        set_string(
            &store,
            &PKEY_AppUserModel_RelaunchDisplayNameResource,
            &identity.display_name,
        )?;
        if let Some(icon) = &identity.icon {
            set_string(
                &store,
                &PKEY_AppUserModel_RelaunchIconResource,
                &format!("{},0", icon.display()),
            )?;
        }
    }
    set_string(&store, &PKEY_AppUserModel_ID, &identity.app_user_model_id)?;
    unsafe { store.Commit() }
}

/// The first icon of `program` at 256 px (or the closest size it has), as RGBA.
pub(crate) fn program_icon(program: &Path) -> io::Result<Image> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, ReleaseDC,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW,
    };

    const SIZE: i32 = 256;
    let no_icon = || io::Error::new(io::ErrorKind::NotFound, "the program has no icon");
    // PrivateExtractIconsW reads a MAX_PATH buffer.
    let mut name = [0u16; 260];
    let wide: Vec<u16> = program
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .collect();
    if wide.len() >= name.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the program's path is too long",
        ));
    }
    name[..wide.len()].copy_from_slice(&wide);

    unsafe {
        let mut icon: HICON = null_mut();
        let mut id = 0u32;
        let count = PrivateExtractIconsW(name.as_ptr(), 0, SIZE, SIZE, &mut icon, &mut id, 1, 0);
        if count == 0 || count == u32::MAX || icon.is_null() {
            return Err(no_icon());
        }
        let mut info: ICONINFO = std::mem::zeroed();
        if GetIconInfo(icon, &mut info) == 0 {
            DestroyIcon(icon);
            return Err(no_icon());
        }
        let mut bitmap: BITMAP = std::mem::zeroed();
        GetObjectW(
            info.hbmColor,
            size_of::<BITMAP>() as i32,
            &mut bitmap as *mut BITMAP as *mut _,
        );
        let (width, height) = (bitmap.bmWidth, bitmap.bmHeight);
        let mut pixels = Vec::new();
        if width > 0 && height > 0 {
            let mut header: BITMAPINFO = std::mem::zeroed();
            header.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            pixels = vec![0u8; width as usize * height as usize * 4];
            let dc = GetDC(null_mut());
            let rows = GetDIBits(
                dc,
                info.hbmColor,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut header,
                DIB_RGB_COLORS,
            );
            ReleaseDC(null_mut(), dc);
            if rows == 0 {
                pixels.clear();
            }
        }
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
        DestroyIcon(icon);
        if pixels.is_empty() {
            return Err(no_icon());
        }
        for pixel in pixels.as_chunks_mut::<4>().0.iter_mut() {
            pixel.swap(0, 2); // BGRA → RGBA
        }
        // An old-style icon carries no alpha: treat it as opaque.
        if pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            for pixel in pixels.as_chunks_mut::<4>().0.iter_mut() {
                pixel[3] = 255;
            }
        }
        Ok(Image::new(width as u32, height as u32, pixels))
    }
}
