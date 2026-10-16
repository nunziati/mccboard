//! A desktop profile's notification-area icon (spec #34, ticket #38, ADR 0008). Closed to the
//! tray, every copy of the desktop app shows the same white icon; mccboard repaints the icon of
//! every profile but the default one with Claude's mark in the account's color and the tooltip
//! `Claude <label>`.
//!
//! - **What** ([`profile_tray_icons`]): for each running profile, the cached tray `.ico`
//!   ([`crate::ProfileIcons::tray`]), the tooltip in the app's language, and the processes
//!   where the app's icon host window may be, the main one first.
//! - **When** ([`ProfileTrayKeeper`]): the app gives no way to read its icon back, so a reset
//!   (on a notification, a state change) can't be seen: the icon is applied again every
//!   [`PROFILE_TRAY_REAPPLY_EVERY`]. Sooner when what it should show changed, when the last
//!   try failed (the app still starting, its icon not there yet), and on every look for
//!   [`PROFILE_TRAY_AFTER_TASKBAR_CREATED`] once Explorer restarted, since the app then adds
//!   its own icon back.
//! - **How**, in `windows_profile_tray`: the profile's main process owns a window of class
//!   `Electron_NotifyIconHostWindow`; its icon is modified with `Shell_NotifyIconW`, by that
//!   window and an icon id found by trying [`ProfileTrayRepaint::id_candidates`] until the
//!   shell accepts one. The id that worked is remembered for that process.
//!
//! Everything here is plain logic, the same on every OS; time is passed in.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{AccountUuid, Language, RunningProfile, TOOLTIP_MAX_UTF16};

/// How often a profile's icon is applied again while nothing says it changed.
///
/// To settle on the laptop: ADR 0008 says the app doesn't reset properties set from outside,
/// ticket #38 expects it resets its tray icon on notifications. If B's icon never turns white
/// again, drop this periodic repaint (keep `TaskbarCreated`); if it does, update ADR 0008.
pub const PROFILE_TRAY_REAPPLY_EVERY: Duration = Duration::from_secs(10);

/// How long after Explorer restarts every look applies the icons again.
pub const PROFILE_TRAY_AFTER_TASKBAR_CREATED: Duration = Duration::from_secs(10);

/// The icon id Electron gives the app's first tray icon (its base id 2, plus 1).
const FIRST_ELECTRON_ID: u32 = 3;

/// The highest icon id tried: an app that made its tray icon again a few times.
const LAST_ID: u32 = 20;

/// What a running desktop profile's notification-area icon should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileTrayIcon {
    pub account: AccountUuid,
    /// The processes whose icon host window it may be, the main process first.
    pub processes: Vec<u32>,
    /// The tray `.ico`: Claude's mark in the account's color, at the small-icon sizes. `None`
    /// when it can't be drawn: then only the tooltip is set.
    pub icon: Option<PathBuf>,
    /// `Claude <label>`, at most [`TOOLTIP_MAX_UTF16`] UTF-16 units.
    pub tooltip: String,
}

/// The notification-area icons of the running desktop profiles, in `language`. `profiles`
/// comes from [`crate::Core::running_desktop_profiles`], which never lists the default
/// profile: its icon is never touched.
pub fn profile_tray_icons(profiles: &[RunningProfile], language: Language) -> Vec<ProfileTrayIcon> {
    profiles
        .iter()
        .map(|profile| {
            let mut processes: Vec<u32> = profile.main_pid.into_iter().collect();
            processes.extend(
                profile
                    .pids
                    .iter()
                    .filter(|&&pid| Some(pid) != profile.main_pid),
            );
            ProfileTrayIcon {
                account: profile.account.clone(),
                processes,
                icon: profile.icons.as_ref().map(|icons| icons.tray.clone()),
                tooltip: profile_tray_tooltip(&profile.label, language),
            }
        })
        .collect()
}

/// The tooltip of a profile's notification-area icon: `Claude <label>`, shortened with "…"
/// to fit the Windows tray.
pub fn profile_tray_tooltip(label: &str, language: Language) -> String {
    // The same in every language so far; a new one has to say.
    let text = match language {
        Language::English | Language::Italian => format!("Claude {label}"),
    };
    if text.encode_utf16().count() <= TOOLTIP_MAX_UTF16 {
        return text;
    }
    let mut short = String::new();
    let mut units = 1; // the "…"
    for c in text.chars() {
        units += c.len_utf16();
        if units > TOOLTIP_MAX_UTF16 {
            break;
        }
        short.push(c);
    }
    short.push('…');
    short
}

/// Where a profile's icon was found: its host window's process and the icon id the shell
/// accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconAddress {
    pub process: u32,
    pub id: u32,
}

/// A profile's icon to apply now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileTrayRepaint {
    pub icon: ProfileTrayIcon,
    /// Where it was found last time, if that process still runs the profile.
    pub remembered: Option<IconAddress>,
}

impl ProfileTrayRepaint {
    /// The icon ids to try, in order, for the host window of `process`: the one remembered
    /// for that process first, then Electron's first id and the ones after it, then the rest.
    pub fn id_candidates(&self, process: u32) -> Vec<u32> {
        let remembered = self
            .remembered
            .filter(|address| address.process == process)
            .map(|address| address.id);
        let mut ids: Vec<u32> = remembered.into_iter().collect();
        for id in (FIRST_ELECTRON_ID..=LAST_ID).chain(1..FIRST_ELECTRON_ID) {
            if Some(id) != remembered {
                ids.push(id);
            }
        }
        ids
    }
}

/// How applying a profile's icon went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepaintOutcome {
    /// The shell accepted the icon at this address.
    Applied(IconAddress),
    /// The host window is there but the shell accepted no id: the app has no tray icon yet.
    NotAccepted,
    /// None of the profile's processes has an icon host window yet.
    NoHostWindow,
}

/// What was last applied to a profile's icon.
#[derive(Debug, Clone)]
struct Kept {
    /// The icon and tooltip, when the shell accepted them; `None` after a failed try.
    applied: Option<(Option<PathBuf>, String)>,
    /// Where the icon was last found, kept across failed tries.
    address: Option<IconAddress>,
    at: Instant,
}

/// Decides which profiles' icons to apply on each look at the running profiles, and remembers
/// how it went.
#[derive(Debug, Clone, Default)]
pub struct ProfileTrayKeeper {
    kept: HashMap<AccountUuid, Kept>,
    taskbar_created: Option<Instant>,
}

impl ProfileTrayKeeper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Explorer restarted at `now` (the `TaskbarCreated` message): the shell lost every icon,
    /// and the apps add theirs back, white.
    pub fn taskbar_created(&mut self, now: Instant) {
        self.taskbar_created = Some(now);
    }

    /// The icons among `icons` to apply at `now`: one never applied, changed, last failed,
    /// applied [`PROFILE_TRAY_REAPPLY_EVERY`] ago or more, or any within
    /// [`PROFILE_TRAY_AFTER_TASKBAR_CREATED`] of Explorer restarting. Profiles no longer
    /// running are forgotten.
    pub fn due(&mut self, icons: &[ProfileTrayIcon], now: Instant) -> Vec<ProfileTrayRepaint> {
        self.kept
            .retain(|account, _| icons.iter().any(|icon| &icon.account == account));
        let restarted = self.taskbar_created.is_some_and(|at| {
            now.saturating_duration_since(at) < PROFILE_TRAY_AFTER_TASKBAR_CREATED
        });
        icons
            .iter()
            .filter_map(|icon| {
                let kept = self.kept.get(&icon.account);
                let remembered = kept
                    .and_then(|k| k.address)
                    .filter(|address| icon.processes.contains(&address.process));
                let due = match kept {
                    None => true,
                    Some(kept) => {
                        let shows = (icon.icon.clone(), icon.tooltip.clone());
                        restarted
                            || remembered.is_none()
                            || kept.applied.as_ref() != Some(&shows)
                            || now.saturating_duration_since(kept.at) >= PROFILE_TRAY_REAPPLY_EVERY
                    }
                };
                due.then(|| ProfileTrayRepaint {
                    icon: icon.clone(),
                    remembered,
                })
            })
            .collect()
    }

    /// How applying `repaint` at `now` went.
    pub fn record(&mut self, repaint: &ProfileTrayRepaint, outcome: RepaintOutcome, now: Instant) {
        let icon = &repaint.icon;
        let kept = match outcome {
            RepaintOutcome::Applied(address) => Kept {
                applied: Some((icon.icon.clone(), icon.tooltip.clone())),
                address: Some(address),
                at: now,
            },
            // Tried again at the next look, from the id remembered for the process.
            RepaintOutcome::NotAccepted | RepaintOutcome::NoHostWindow => Kept {
                applied: None,
                address: self.kept.get(&icon.account).and_then(|k| k.address),
                at: now,
            },
        };
        self.kept.insert(icon.account.clone(), kept);
    }
}
