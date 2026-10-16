//! The tray's "Open profile" submenu (ticket #39): renders the core's [`TrayProfiles`], each
//! profile with a dot in its account's color, and opens the one clicked. A profile that runs
//! already opens all the same: the app brings it to the front.

use mccboard_core::{AccountUuid, Color, TrayProfile, TrayProfiles};
use tauri::AppHandle;
use tauri::image::Image;
use tauri::menu::{IconMenuItem, IsMenuItem, Submenu};

/// Menu ids: `open-profile:default`, or `open-profile:<account uuid>`.
const PREFIX: &str = "open-profile:";
const DEFAULT: &str = "default";

/// The side of a profile's color dot, in pixels.
const DOT: u32 = 16;

/// The submenu, or `None` when there is no profile to open (off Windows).
pub(super) fn submenu(
    app: &AppHandle,
    profiles: &TrayProfiles,
) -> tauri::Result<Option<Submenu<tauri::Wry>>> {
    if profiles.profiles.is_empty() {
        return Ok(None);
    }
    let items = profiles
        .profiles
        .iter()
        .map(|profile| {
            let title = super::escape(&profile.title);
            let dot = Some(dot(&profile.color));
            match menu_id(profile) {
                Some(id) => IconMenuItem::with_id(app, id, title, true, dot, None::<&str>),
                // Nothing it could open but the wrong profile: shown, disabled.
                None => IconMenuItem::new(app, title, false, dot, None::<&str>),
            }
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let items: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item as _).collect();
    Submenu::with_items(app, super::escape(&profiles.title), true, &items).map(Some)
}

/// The menu id that opens `profile`: the default profile by its own id, any other by its
/// account. `None` for a profile other than the default one without an account, which no id
/// could open: its entry is disabled rather than opening the default profile.
fn menu_id(profile: &TrayProfile) -> Option<String> {
    match (&profile.account, profile.default) {
        (_, true) => Some(format!("{PREFIX}{DEFAULT}")),
        (Some(account), false) => Some(format!("{PREFIX}{account}")),
        (None, false) => None,
    }
}

/// Handles a click on an "Open profile" entry; `false` for any other menu id. A profile with
/// no account signed in yet also shows the Instances window, with its first sign-in guide.
pub(super) fn on_menu_event(app: &AppHandle, id: &str) -> bool {
    let Some(which) = id.strip_prefix(PREFIX) else {
        return false;
    };
    let account = (which != DEFAULT).then(|| AccountUuid::new(which));
    crate::desktop::open_in_background(app, account, |_| {});
    true
}

/// A filled circle in `color`, antialiased at its edge, on a transparent square.
fn dot(color: &Color) -> Image<'static> {
    let [r, g, b] = color.rgb();
    let center = (DOT as f32 - 1.0) / 2.0;
    let radius = DOT as f32 / 2.0 - 1.5;
    let mut rgba = Vec::with_capacity((DOT * DOT * 4) as usize);
    for y in 0..DOT {
        for x in 0..DOT {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[r, g, b, (coverage * 255.0).round() as u8]);
        }
    }
    Image::new_owned(rgba, DOT, DOT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(default: bool, account: Option<&str>) -> TrayProfile {
        TrayProfile {
            default,
            account: account.map(AccountUuid::new),
            title: "B".into(),
            color: "#3366cc".parse().unwrap(),
        }
    }

    #[test]
    fn each_entry_opens_its_own_profile_and_one_without_an_account_none() {
        let uuid = "22222222-2222-4222-8222-222222222222";
        assert_eq!(
            menu_id(&entry(true, Some(uuid))).as_deref(),
            Some("open-profile:default")
        );
        assert_eq!(
            menu_id(&entry(true, None)).as_deref(),
            Some("open-profile:default")
        );
        assert_eq!(
            menu_id(&entry(false, Some(uuid))),
            Some(format!("open-profile:{uuid}"))
        );
        assert_eq!(menu_id(&entry(false, None)), None);
    }

    #[test]
    fn the_dot_is_the_color_in_the_middle_and_clear_in_the_corners() {
        let image = dot(&"#3366cc".parse().unwrap());
        let rgba = image.rgba();
        let pixel = |x: u32, y: u32| {
            let i = ((y * DOT + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        assert_eq!(pixel(DOT / 2, DOT / 2), [0x33, 0x66, 0xcc, 255]);
        assert_eq!(pixel(0, 0)[3], 0);
        assert_eq!(pixel(DOT - 1, DOT - 1)[3], 0);
    }
}
