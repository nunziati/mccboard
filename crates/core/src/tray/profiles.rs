//! The tray's "Open profile" menu (ticket #39): every desktop profile, the default one first,
//! each with its account's color. Kept apart from the usage view, which lists no processes:
//! this one does, through the desktop profiles' view.

use serde::Serialize;

use crate::{AccountUuid, Color, DesktopProfile, Language};

/// The "Open profile" submenu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayProfiles {
    /// The submenu's label.
    pub title: String,
    /// Every desktop profile, the default one first; empty where the app doesn't run (off
    /// Windows), and the tray then shows no submenu.
    pub profiles: Vec<TrayProfile>,
}

/// One desktop profile in the tray menu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayProfile {
    pub default: bool,
    /// The profile's account; for the default profile, the primary instance's, if any.
    pub account: Option<AccountUuid>,
    /// The menu entry: the account's label, with "(default)" after the default profile's.
    pub title: String,
    pub color: Color,
    /// A profile other than the default one with no account signed in yet: opening it should
    /// show the first sign-in guide.
    pub needs_sign_in: bool,
}

pub(crate) fn view(profiles: Vec<DesktopProfile>, language: Language) -> TrayProfiles {
    let pick = |en: &'static str, it: &'static str| match language {
        Language::English => en,
        Language::Italian => it,
    };
    let profiles = profiles
        .into_iter()
        .map(|p| {
            let title = match (&p.label, p.default) {
                (Some(label), true) => format!("{label} ({})", pick("default", "predefinito")),
                (None, true) => pick("Default profile", "Profilo predefinito").to_owned(),
                (Some(label), false) => label.clone(),
                (None, false) => p
                    .account
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            };
            TrayProfile {
                default: p.default,
                needs_sign_in: !p.default && p.observed.account.is_none(),
                account: p.account,
                title,
                color: p.color,
            }
        })
        .collect();
    TrayProfiles {
        title: pick("Open profile", "Apri profilo").to_owned(),
        profiles,
    }
}
