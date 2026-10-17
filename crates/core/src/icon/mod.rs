//! Desktop profile icons (spec #34, tickets #37 and #38). Pure image code, on every OS; the
//! Windows code that puts them on windows, shortcuts and the tray lives elsewhere.
//!
//! - **Sources**: the desktop app's own logos, found in its package's install folder
//!   ([`PackageLogos`]), or images the caller got otherwise (the exe's icon, the app's tray
//!   icon). Anthropic's logo is never shipped with mccboard.
//! - **Taskbar icon** (#37): the plated logo trimmed to its visible pixels (the tile logos
//!   carry wide transparent padding), its saturated pixels recolored to the account's color,
//!   written as an `.ico` with every size Windows asks for.
//! - **Tray mark** (#38): the mark alone, in the account's color, on a transparent background.
//! - **Cache** (`~/.mccboard/state/icons/`): one file per account and color, so a new color
//!   gets a new file name and Explorer's icon cache can't serve the old picture.

mod ico;
mod logo;
mod package;
mod pixels;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use logo::{LogoStyle, mccboard_logo};
pub use package::PackageLogos;
use pixels::MarkIn;

use crate::{AccountUuid, Color, Error, Paths};

/// The frames of the taskbar and shortcut icon, in px.
const APP_SIZES: [u32; 12] = [16, 20, 24, 30, 32, 36, 40, 48, 64, 72, 96, 256];

/// The frames of the tray icon (small-icon sizes at 100 % to 300 % scaling), in px.
const TRAY_SIZES: [u32; 6] = [16, 20, 24, 32, 40, 48];

/// An RGBA8 image, rows top-down: `rgba` holds `width × height × 4` bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}×{})", self.width, self.height)
    }
}

impl Image {
    /// An image of `width × height` pixels from its RGBA8 bytes, rows top-down.
    ///
    /// # Panics
    /// If `rgba` doesn't hold exactly `width × height × 4` bytes.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Image {
        assert_eq!(
            rgba.len(),
            width as usize * height as usize * 4,
            "RGBA8 bytes of a {width}×{height} image"
        );
        Image {
            width,
            height,
            rgba,
        }
    }

    /// Decodes a PNG of any color type and bit depth into RGBA8. The error says why, in
    /// words from the decoder.
    pub fn from_png(bytes: &[u8]) -> Result<Image, String> {
        let mut decoder = png::Decoder::new(io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| "image too large".to_owned())?;
        let mut buf = vec![0; size];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        buf.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buf
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return Err("unexpanded palette".to_owned()),
        };
        if rgba.len() != info.width as usize * info.height as usize * 4 {
            return Err("unexpected pixel layout".to_owned());
        }
        Ok(Image::new(info.width, info.height, rgba))
    }

    /// Reads and decodes the PNG file at `path` ([`Error::IconSourceUnreadable`] otherwise).
    pub fn load_png(path: &Path) -> Result<Image, Error> {
        let unreadable = |detail: String| Error::IconSourceUnreadable {
            path: path.to_path_buf(),
            detail,
        };
        let bytes = fs::read(path).map_err(|e| unreadable(e.to_string()))?;
        Image::from_png(&bytes).map_err(unreadable)
    }

    /// The image as PNG bytes (RGBA, 8 bits per channel).
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let written = encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&self.rgba));
        written.expect("encoding a valid RGBA8 image in memory doesn't fail");
        out
    }

    /// Its visible pixels (alpha above 16) centred in a transparent square with no margin.
    pub fn trimmed(&self) -> Image {
        pixels::trim_square(self)
    }

    /// This square image resampled to `side × side` (area average, premultiplied alpha).
    ///
    /// # Panics
    /// If the image isn't square.
    pub fn resized(&self, side: u32) -> Image {
        pixels::resize(self, side)
    }

    /// Its saturated pixels (HSL saturation ≥ 0.3) in `color`'s hue and saturation, each
    /// keeping its lightness offset from Claude's orange (`#D97757`). White, grey and alpha
    /// stay as they are.
    pub fn recolored(&self, color: &Color) -> Image {
        let mut out = self.clone();
        pixels::recolor(&mut out, color.rgb());
        out
    }
}

/// The images a desktop profile's icons are drawn from. At least one is needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IconSources {
    /// The logo on its tile: the package's plated logo, or the exe's icon. The taskbar icon is
    /// drawn from it, else from `unplated`.
    pub plated: Option<Image>,
    /// The mark alone on a transparent background: the package's unplated logo, or the app's
    /// tray icon. The tray mark is drawn from its visible pixels, else from `plated`'s white
    /// ones.
    pub unplated: Option<Image>,
}

impl IconSources {
    /// The logos of the package installed in `install_dir` ([`PackageLogos::find`]), decoded.
    /// A logo that is found but can't be read is an [`Error::IconSourceUnreadable`]; none
    /// found leaves both `None`.
    pub fn from_package(install_dir: &Path) -> Result<IconSources, Error> {
        let logos = PackageLogos::find(install_dir);
        let load = |path: Option<PathBuf>| path.as_deref().map(Image::load_png).transpose();
        Ok(IconSources {
            plated: load(logos.plated)?,
            unplated: load(logos.unplated)?,
        })
    }

    /// The taskbar and shortcut icon in `color`: the logo trimmed, recolored, as an `.ico`
    /// with frames at 16 to 96 px (32-bit DIBs) and at 256 px (PNG).
    pub fn app_ico(&self, color: &Color) -> Result<Vec<u8>, Error> {
        let source = self
            .plated
            .as_ref()
            .or(self.unplated.as_ref())
            .ok_or(Error::NoIconSource)?;
        let logo = source.trimmed().recolored(color);
        let frames: Vec<Image> = APP_SIZES.iter().map(|&side| logo.resized(side)).collect();
        Ok(ico::write(&frames))
    }

    /// Claude's mark alone, every pixel in `color`, on a transparent background, trimmed to
    /// fill a `side × side` square.
    pub fn tray_mark(&self, color: &Color, side: u32) -> Result<Image, Error> {
        Ok(self.mark(color)?.resized(side))
    }

    /// The tray icon in `color`: [`IconSources::tray_mark`] as an `.ico` with the small-icon
    /// sizes, 16 to 48 px.
    pub fn tray_ico(&self, color: &Color) -> Result<Vec<u8>, Error> {
        let mark = self.mark(color)?;
        let frames: Vec<Image> = TRAY_SIZES.iter().map(|&side| mark.resized(side)).collect();
        Ok(ico::write(&frames))
    }

    /// The mark, trimmed, at the source's resolution.
    fn mark(&self, color: &Color) -> Result<Image, Error> {
        let (source, mark_in) = match (&self.unplated, &self.plated) {
            (Some(unplated), _) => (unplated, MarkIn::Alpha),
            (None, Some(plated)) => (plated, MarkIn::White),
            (None, None) => return Err(Error::NoIconSource),
        };
        Ok(pixels::mark(source, mark_in, color.rgb()).trimmed())
    }
}

/// Where a desktop profile's icons are cached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileIcons {
    /// The taskbar and shortcut icon: `<account>-<rrggbb>.ico`.
    pub app: PathBuf,
    /// The tray icon: `<account>-<rrggbb>-tray.ico`.
    pub tray: PathBuf,
}

/// The cache of desktop profile icons in `~/.mccboard/state/icons/`, named by account and
/// color. Only files named that way are ever touched.
#[derive(Debug, Clone)]
pub struct IconCache {
    dir: PathBuf,
}

impl IconCache {
    pub fn new(paths: &Paths) -> IconCache {
        IconCache {
            dir: paths.state_dir().join("icons"),
        }
    }

    /// Where `account`'s icons in `color` are, whether they exist yet or not.
    pub fn icons(&self, account: &AccountUuid, color: &Color) -> ProfileIcons {
        let stem = format!("{}-{}", file_part(account), &color.as_str()[1..]);
        ProfileIcons {
            app: self.dir.join(format!("{stem}.ico")),
            tray: self.dir.join(format!("{stem}-tray.ico")),
        }
    }

    /// `account`'s icons in `color`, drawn from `sources` (called only when one is missing)
    /// and cached; the account's icons in any other color are removed. Each file is written
    /// whole or not at all. Nothing is written when `sources` fails.
    pub fn ensure(
        &self,
        account: &AccountUuid,
        color: &Color,
        sources: impl FnOnce() -> Result<IconSources, Error>,
    ) -> Result<ProfileIcons, Error> {
        let icons = self.icons(account, color);
        if !(icons.app.is_file() && icons.tray.is_file()) {
            let sources = sources()?;
            let app = sources.app_ico(color)?;
            let tray = sources.tray_ico(color)?;
            fs::create_dir_all(&self.dir).map_err(|source| Error::IconUnwritable {
                path: self.dir.clone(),
                source,
            })?;
            write_whole(&icons.app, &app)?;
            write_whole(&icons.tray, &tray)?;
        }
        let current = [&icons.app, &icons.tray].map(|p| p.file_name().map(|n| n.to_owned()));
        self.remove_where(account, |name| {
            !current.iter().flatten().any(|keep| keep == name)
        })?;
        Ok(icons)
    }

    /// Removes every cached icon of `account`, in any color (when its desktop profile goes).
    pub fn remove(&self, account: &AccountUuid) -> Result<(), Error> {
        self.remove_where(account, |_| true)
    }

    /// Removes `account`'s cached icons whose file name passes `remove`.
    fn remove_where(
        &self,
        account: &AccountUuid,
        remove: impl Fn(&std::ffi::OsStr) -> bool,
    ) -> Result<(), Error> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Ok(());
        };
        let prefix = format!("{}-", file_part(account));
        for entry in entries.flatten() {
            let name = entry.file_name();
            let ours = name
                .to_str()
                .and_then(|n| n.strip_prefix(&prefix))
                .is_some_and(is_color_file);
            if ours && remove(&name) {
                let path = entry.path();
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(source) => return Err(Error::IconUnwritable { path, source }),
                }
            }
        }
        Ok(())
    }
}

/// Whether `rest` is `rrggbb.ico` or `rrggbb-tray.ico`: what follows `<account>-` in a cached
/// icon's name.
fn is_color_file(rest: &str) -> bool {
    let Some(suffix) = rest.get(6..) else {
        return false;
    };
    rest[..6].chars().all(|c| c.is_ascii_hexdigit()) && matches!(suffix, ".ico" | "-tray.ico")
}

/// The account UUID as a file name part: anything but ASCII letters, digits and `-` (a
/// hand-edited `config.toml`) becomes `_`.
fn file_part(account: &AccountUuid) -> String {
    account
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over `path`, so a
/// reader (Explorer, the shell) never sees half an icon.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    crate::atomic_file::write(path, bytes).map_err(|source| Error::IconUnwritable {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_icon_names() {
        assert!(is_color_file("2563eb.ico"));
        assert!(is_color_file("2563eb-tray.ico"));
        assert!(!is_color_file("2563eb.ico.tmp"));
        assert!(!is_color_file("2563e.ico"));
        assert!(!is_color_file("ico"));
        assert!(!is_color_file("é563eb.ico"));
        assert_eq!(file_part(&AccountUuid::new("a/b\\c-1")), "a_b_c-1");
    }
}
