//! A desktop profile's notification-area icon (spec #34, ticket #38): what it shows (the
//! colored mark, `Claude <label>` in each language) and when it is applied again (every few
//! seconds, sooner when it changed or failed, and after Explorer restarts). The Windows calls
//! that apply it are checked by hand on the laptop.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mccboard_core::{
    AccountUuid, IconAddress, Language, PROFILE_TRAY_AFTER_TASKBAR_CREATED,
    PROFILE_TRAY_REAPPLY_EVERY, ProfileIcons, ProfileTrayIcon, ProfileTrayKeeper, RepaintOutcome,
    RunningProfile, TOOLTIP_MAX_UTF16, WindowIdentity, profile_tray_icons, profile_tray_tooltip,
};

const UUID_B: &str = "22222222-2222-4222-8222-222222222222";
const UUID_C: &str = "33333333-3333-4333-8333-333333333333";

fn running(uuid: &str, label: &str, pids: &[u32], main_pid: Option<u32>) -> RunningProfile {
    RunningProfile {
        account: AccountUuid::new(uuid),
        label: label.to_owned(),
        color: "#2563eb".parse().unwrap(),
        pids: pids.to_vec(),
        main_pid,
        icons: Some(ProfileIcons {
            app: PathBuf::from(format!("{uuid}-2563eb.ico")),
            tray: PathBuf::from(format!("{uuid}-2563eb-tray.ico")),
        }),
        identity: WindowIdentity {
            app_user_model_id: format!("profile.{uuid}"),
            display_name: format!("Claude {label}"),
            relaunch_command: None,
            icon: None,
        },
    }
}

fn icon(uuid: &str, label: &str, pid: u32) -> ProfileTrayIcon {
    ProfileTrayIcon {
        account: AccountUuid::new(uuid),
        processes: vec![pid],
        icon: Some(PathBuf::from(format!("{uuid}-2563eb-tray.ico"))),
        tooltip: format!("Claude {label}"),
    }
}

fn applied(pid: u32, id: u32) -> RepaintOutcome {
    RepaintOutcome::Applied(IconAddress { process: pid, id })
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// --- What the icon shows ---------------------------------------------------------------------

#[test]
fn the_icon_is_the_tray_mark_with_the_label_in_each_language() {
    let profiles = [running(UUID_B, "B", &[20, 21], Some(20))];
    for language in [Language::English, Language::Italian] {
        let icons = profile_tray_icons(&profiles, language);
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].account, AccountUuid::new(UUID_B));
        assert_eq!(
            icons[0].icon,
            Some(PathBuf::from(format!("{UUID_B}-2563eb-tray.ico"))),
            "the tray .ico, not the taskbar one"
        );
        assert_eq!(icons[0].tooltip, "Claude B");
    }
}

#[test]
fn the_host_window_is_looked_for_in_the_main_process_first() {
    let profiles = [
        running(UUID_B, "B", &[18, 20, 21], Some(20)),
        // The main process can't be told: every process, in order.
        running(UUID_C, "C", &[30, 31], None),
    ];
    let icons = profile_tray_icons(&profiles, Language::English);
    assert_eq!(icons[0].processes, [20, 18, 21]);
    assert_eq!(icons[1].processes, [30, 31]);
}

#[test]
fn a_profile_without_icons_still_gets_its_tooltip() {
    let mut profile = running(UUID_B, "B", &[20], Some(20));
    profile.icons = None;
    let icons = profile_tray_icons(&[profile], Language::Italian);
    assert_eq!(icons[0].icon, None);
    assert_eq!(icons[0].tooltip, "Claude B");
}

#[test]
fn a_long_label_is_shortened_to_fit_the_windows_tray() {
    let label = "é".repeat(200);
    let tooltip = profile_tray_tooltip(&label, Language::English);
    assert_eq!(tooltip.encode_utf16().count(), TOOLTIP_MAX_UTF16);
    assert!(tooltip.starts_with("Claude é"));
    assert!(tooltip.ends_with('…'));
    // Surrogate pairs aren't split.
    let tooltip = profile_tray_tooltip(&"🙂".repeat(100), Language::English);
    assert!(tooltip.encode_utf16().count() <= TOOLTIP_MAX_UTF16);
    assert!(tooltip.ends_with("🙂…"));
}

// --- Which icon ids are tried -----------------------------------------------------------------

#[test]
fn electrons_first_id_is_tried_first_then_the_others() {
    let mut keeper = ProfileTrayKeeper::new();
    let now = Instant::now();
    let due = keeper.due(&[icon(UUID_B, "B", 20)], now);
    let ids = due[0].id_candidates(20);
    assert_eq!(ids[0], 3, "Electron's first tray icon");
    assert_eq!(&ids[..3], &[3, 4, 5]);
    assert!(ids.contains(&1) && ids.contains(&2));
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "no id twice");
}

#[test]
fn the_id_that_worked_is_tried_first_for_the_same_process_only() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let b = icon(UUID_B, "B", 20);
    let due = keeper.due(std::slice::from_ref(&b), start);
    keeper.record(&due[0], applied(20, 7), start);

    let due = keeper.due(std::slice::from_ref(&b), start + PROFILE_TRAY_REAPPLY_EVERY);
    assert_eq!(due[0].remembered, Some(IconAddress { process: 20, id: 7 }));
    let ids = due[0].id_candidates(20);
    assert_eq!(&ids[..2], &[7, 3]);
    assert_eq!(ids.iter().filter(|&&id| id == 7).count(), 1);
    // Another process (a host window found elsewhere): the id isn't its.
    assert_eq!(due[0].id_candidates(21)[0], 3);
}

#[test]
fn a_restarted_profile_has_its_id_found_again() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let due = keeper.due(&[icon(UUID_B, "B", 20)], start);
    keeper.record(&due[0], applied(20, 3), start);

    // B was closed and opened again: a new main process, right away.
    let due = keeper.due(&[icon(UUID_B, "B", 40)], start + secs(1));
    assert_eq!(due.len(), 1, "a new process is painted at once");
    assert_eq!(due[0].remembered, None);
}

// --- When the icon is applied -----------------------------------------------------------------

#[test]
fn an_applied_icon_is_applied_again_every_few_seconds_not_on_every_look() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let b = [icon(UUID_B, "B", 20)];

    let due = keeper.due(&b, start);
    assert_eq!(due.len(), 1, "a new profile is painted at once");
    keeper.record(&due[0], applied(20, 3), start);

    assert!(
        keeper.due(&b, start + secs(5)).is_empty(),
        "nothing changed"
    );
    let due = keeper.due(&b, start + PROFILE_TRAY_REAPPLY_EVERY);
    assert_eq!(due.len(), 1, "the app may have reset it meanwhile");
    keeper.record(&due[0], applied(20, 3), start + PROFILE_TRAY_REAPPLY_EVERY);
    assert!(
        keeper
            .due(&b, start + PROFILE_TRAY_REAPPLY_EVERY + secs(5))
            .is_empty()
    );
}

#[test]
fn a_new_label_or_color_is_applied_at_once() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let due = keeper.due(&[icon(UUID_B, "B", 20)], start);
    keeper.record(&due[0], applied(20, 3), start);

    let renamed = icon(UUID_B, "Bee", 20);
    assert_eq!(keeper.due(&[renamed], start + secs(1)).len(), 1);

    let mut recolored = icon(UUID_B, "B", 20);
    recolored.icon = Some(PathBuf::from(format!("{UUID_B}-16a34a-tray.ico")));
    assert_eq!(keeper.due(&[recolored], start + secs(1)).len(), 1);
}

#[test]
fn a_failed_try_is_retried_at_the_next_look() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let b = [icon(UUID_B, "B", 20)];

    let due = keeper.due(&b, start);
    keeper.record(&due[0], RepaintOutcome::NoHostWindow, start);
    let due = keeper.due(&b, start + secs(1));
    assert_eq!(due.len(), 1, "the app is still starting");
    keeper.record(&due[0], RepaintOutcome::NotAccepted, start + secs(1));
    let due = keeper.due(&b, start + secs(2));
    assert_eq!(due.len(), 1, "its tray icon isn't there yet");
    keeper.record(&due[0], applied(20, 3), start + secs(2));
    assert!(keeper.due(&b, start + secs(3)).is_empty());
}

#[test]
fn a_failed_try_keeps_the_id_remembered() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let b = [icon(UUID_B, "B", 20)];
    let due = keeper.due(&b, start);
    keeper.record(&due[0], applied(20, 5), start);
    let due = keeper.due(&b, start + PROFILE_TRAY_REAPPLY_EVERY);
    keeper.record(
        &due[0],
        RepaintOutcome::NotAccepted,
        start + PROFILE_TRAY_REAPPLY_EVERY,
    );

    let due = keeper.due(&b, start + PROFILE_TRAY_REAPPLY_EVERY + secs(1));
    assert_eq!(due[0].remembered, Some(IconAddress { process: 20, id: 5 }));
}

#[test]
fn after_explorer_restarts_every_look_applies_the_icons_for_a_while() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let b = [icon(UUID_B, "B", 20)];
    let due = keeper.due(&b, start);
    keeper.record(&due[0], applied(20, 3), start);

    let restart = start + secs(2);
    keeper.taskbar_created(restart);
    // The app adds its own white icon back, maybe after mccboard repainted: again and again.
    for after in [1, 2, 4] {
        let now = restart + secs(after);
        let due = keeper.due(&b, now);
        assert_eq!(due.len(), 1, "{after} s after the restart");
        keeper.record(&due[0], applied(20, 3), now);
    }
    let calm = restart + PROFILE_TRAY_AFTER_TASKBAR_CREATED;
    assert!(keeper.due(&b, calm).is_empty(), "then calm again");
    let last = restart + secs(4);
    assert_eq!(
        keeper.due(&b, last + PROFILE_TRAY_REAPPLY_EVERY).len(),
        1,
        "the regular reapply"
    );
}

#[test]
fn profiles_no_longer_running_are_forgotten() {
    let mut keeper = ProfileTrayKeeper::new();
    let start = Instant::now();
    let both = [icon(UUID_B, "B", 20), icon(UUID_C, "C", 30)];
    for repaint in keeper.due(&both, start) {
        let pid = repaint.icon.processes[0];
        keeper.record(&repaint, applied(pid, 3), start);
    }
    assert!(keeper.due(&both, start + secs(1)).is_empty());

    // C closed; B stays applied.
    assert!(keeper.due(&both[..1], start + secs(2)).is_empty());
    // C opened again from the same process id (reused): painted again, as new.
    let due = keeper.due(&both, start + secs(3));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].icon.account, AccountUuid::new(UUID_C));
    assert_eq!(due[0].remembered, None);
}
