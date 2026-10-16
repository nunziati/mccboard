//! The tray's "Open profile" submenu (ticket #39): renders the core's [`TrayProfiles`], each
//! profile with a dot in its account's color, and opens the one clicked. A profile that runs
//! already opens all the same: the app brings it to the front.

use mccboard_core::{AccountUuid, Color, Core, TrayProfiles};
use tauri::image::Image;
use tauri::menu::{IconMenuItem, IsMenuItem, Submenu};
use tauri::{AppHandle, Manager};

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
            let id = match (&profile.account, profile.default) {
                (_, true) | (None, _) => format!("{PREFIX}{DEFAULT}"),
                (Some(account), false) => format!("{PREFIX}{account}"),
            };
            IconMenuItem::with_id(
                app,
                id,
                super::escape(&profile.title),
                true,
                Some(dot(&profile.color)),
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let items: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item as _).collect();
    Submenu::with_items(app, super::escape(&profiles.title), true, &items).map(Some)
}

/// Handles a click on an "Open profile" entry; `false` for any other menu id.
pub(super) fn on_menu_event(app: &AppHandle, id: &str) -> bool {
    let Some(which) = id.strip_prefix(PREFIX) else {
        return false;
    };
    let account = (which != DEFAULT).then(|| AccountUuid::new(which));
    let app = app.clone();
    // Finding and starting the app takes a moment: not on the main thread.
    let spawned = std::thread::Builder::new()
        .name("mccboard-open-profile".into())
        .spawn(move || open(&app, account.as_ref()));
    if let Err(error) = spawned {
        eprintln!("mccboard: cannot open the desktop profile: {error}");
    }
    true
}

/// Opens the profile, then shows the Instances window, with its first sign-in guide, when the
/// profile has no account signed in yet.
fn open(app: &AppHandle, account: Option<&AccountUuid>) {
    let core = app.state::<Core>();
    if let Err(error) = crate::desktop::open(&core, account) {
        eprintln!("mccboard: cannot open the desktop profile: {error}");
        return;
    }
    let needs_sign_in = core
        .tray_profiles(super::language(&core))
        .is_ok_and(|menu| {
            menu.profiles
                .iter()
                .any(|p| !p.default && p.account.as_ref() == account && p.needs_sign_in)
        });
    if needs_sign_in {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || super::open_instances(&handle));
    }
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
