//! The Claude desktop app's logos inside its MSIX package's install folder.
//!
//! `AppxManifest.xml` names the taskbar logo (`Square44x44Logo="Assets\Square44x44Logo.png"`),
//! but the named file usually doesn't exist as such: Windows picks among qualified variants
//! next to it, with qualifiers in the file name (`Square44x44Logo.targetsize-256_altform-
//! unplated.png`) or as folder names (`Assets\scale-200\Square44x44Logo.png`):
//! - `targetsize-N`: N px; `scale-N`: 44·N/100 px;
//! - no `altform`: plated, drawn on its tile; `altform-unplated`: the mark as the taskbar
//!   shows it, on a transparent background; `altform-lightunplated` is for the light theme;
//! - `contrast-*`: high contrast, never used here.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// The manifest file at the root of a package's install folder.
const MANIFEST: &str = "AppxManifest.xml";

/// The manifest attribute naming the taskbar logo.
const LOGO_ATTRIBUTE: &str = "Square44x44Logo=\"";

/// The side, in px, of the logo at scale 100.
const LOGO_SIDE: u32 = 44;

/// How deep qualifier folders are looked for under the logo's folder.
const MAX_QUALIFIER_FOLDERS: usize = 3;

/// Resource qualifier names Windows knows (lowercase); a folder named otherwise isn't one.
const QUALIFIERS: &[&str] = &[
    "altform",
    "configuration",
    "contrast",
    "devicefamily",
    "dxfeaturelevel",
    "dxfl",
    "homeregion",
    "lang",
    "language",
    "layoutdir",
    "layoutdirection",
    "scale",
    "targetsize",
    "theme",
];

/// The desktop app's taskbar logos found in its package's install folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PackageLogos {
    /// The plated logo (on its tile): for the taskbar and shortcut icon.
    pub plated: Option<PathBuf>,
    /// The unplated logo (the mark alone, transparent background): for the tray mark.
    pub unplated: Option<PathBuf>,
}

impl PackageLogos {
    /// The logos of the package installed in `install_dir`, as its `AppxManifest.xml` names
    /// them: for each of plated and unplated, the variant with the largest `targetsize`, else
    /// the largest `scale`, else the file as named; never a high-contrast one. Nothing found
    /// (no manifest, no logo, a manifest path leaving the folder) leaves a field `None`.
    /// Reads the manifest and lists folders; opens no image.
    pub fn find(install_dir: &Path) -> PackageLogos {
        let Some(logo) = manifest_logo(install_dir) else {
            return PackageLogos::default();
        };
        let Some((stem, extension)) = logo
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.rsplit_once('.'))
        else {
            return PackageLogos::default();
        };
        let folder = logo.parent().unwrap_or(install_dir);
        let mut variants = Vec::new();
        collect(folder, stem, extension, &BTreeMap::new(), 0, &mut variants);
        variants.sort_by(|a, b| a.path.cmp(&b.path));

        let best = |unplated: bool| {
            variants
                .iter()
                .filter(|v| !v.qualifiers.contains_key("contrast"))
                .filter(|v| match v.qualifiers.get("altform").map(String::as_str) {
                    None => !unplated,
                    Some("unplated") => unplated,
                    Some(_) => false,
                })
                .fold(None::<&Variant>, |best, v| match best {
                    Some(b) if b.rank() >= v.rank() => Some(b),
                    _ => Some(v),
                })
                .map(|v| v.path.clone())
        };
        PackageLogos {
            plated: best(false),
            unplated: best(true),
        }
    }
}

/// The logo path the manifest names, inside `install_dir`.
fn manifest_logo(install_dir: &Path) -> Option<PathBuf> {
    let manifest = fs::read(install_dir.join(MANIFEST)).ok()?;
    let manifest = String::from_utf8_lossy(&manifest);
    let start = manifest.find(LOGO_ATTRIBUTE)? + LOGO_ATTRIBUTE.len();
    let value = &manifest[start..start + manifest[start..].find('"')?];
    let relative: PathBuf = value.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    let inside = relative.components().count() > 0
        && relative
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    inside.then(|| install_dir.join(relative))
}

/// A qualified variant of the logo.
struct Variant {
    path: PathBuf,
    qualifiers: BTreeMap<String, String>,
}

impl Variant {
    /// Larger is better: any `targetsize` beats any `scale`, which beats the bare file.
    fn rank(&self) -> (u8, u32) {
        let number = |key: &str| self.qualifiers.get(key).and_then(|v| v.parse::<u32>().ok());
        if let Some(size) = number("targetsize") {
            (2, size)
        } else if let Some(scale) = number("scale") {
            (1, LOGO_SIDE * scale / 100)
        } else {
            (0, LOGO_SIDE)
        }
    }
}

/// Every variant of `<stem>.<extension>` in `folder` and its qualifier folders.
fn collect(
    folder: &Path,
    stem: &str,
    extension: &str,
    inherited: &BTreeMap<String, String>,
    depth: usize,
    out: &mut Vec<Variant>,
) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_QUALIFIER_FOLDERS
                && let Some(qualifiers) = qualifiers(&name)
            {
                let mut all = inherited.clone();
                all.extend(qualifiers);
                collect(&entry.path(), stem, extension, &all, depth + 1, out);
            }
        } else if kind.is_file()
            && let Some(qualifiers) = file_qualifiers(&name, stem, extension)
        {
            let mut all = inherited.clone();
            all.extend(qualifiers);
            out.push(Variant {
                path: entry.path(),
                qualifiers: all,
            });
        }
    }
}

/// The qualifiers of the file `name` if it is a variant of `<stem>.<extension>`: none for
/// the file itself, those between the stem and the extension otherwise. Case-insensitive.
fn file_qualifiers(name: &str, stem: &str, extension: &str) -> Option<BTreeMap<String, String>> {
    let lower = name.to_lowercase();
    let middle = lower
        .strip_prefix(&stem.to_lowercase())?
        .strip_suffix(&extension.to_lowercase())?
        .strip_suffix('.')?;
    match middle.strip_prefix('.') {
        None if middle.is_empty() => Some(BTreeMap::new()),
        Some(qualified) => qualifiers(qualified),
        None => None,
    }
}

/// `key-value` pairs joined by `_` (`targetsize-256_altform-unplated`), keys known to Windows.
fn qualifiers(text: &str) -> Option<BTreeMap<String, String>> {
    text.to_lowercase()
        .split('_')
        .map(|pair| {
            let (key, value) = pair.split_once('-')?;
            (QUALIFIERS.contains(&key) && !value.is_empty())
                .then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_their_qualifiers() {
        let q = |name| file_qualifiers(name, "Square44x44Logo", "png");
        assert_eq!(q("Square44x44Logo.png"), Some(BTreeMap::new()));
        assert_eq!(
            q("square44x44logo.TargetSize-256_altform-unplated.PNG"),
            Some(BTreeMap::from([
                ("targetsize".to_owned(), "256".to_owned()),
                ("altform".to_owned(), "unplated".to_owned()),
            ]))
        );
        assert_eq!(q("Square44x44Logo.old.png"), None);
        assert_eq!(q("Square44x44LogoX.png"), None);
        assert_eq!(q("Square44x44Logo.scale-200.jpg"), None);
        assert_eq!(q("Square150x150Logo.scale-200.png"), None);
    }

    #[test]
    fn folder_names_are_qualifiers_only_when_windows_knows_them() {
        assert!(qualifiers("scale-200").is_some());
        assert!(qualifiers("contrast-black_scale-100").is_some());
        assert!(qualifiers("images").is_none());
        assert!(qualifiers("en-us").is_none());
    }
}
