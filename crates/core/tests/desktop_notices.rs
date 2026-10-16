//! Noticing a desktop profile the app reopened on its own (spec #34, ticket #40): successive
//! process lists fed through the fake environment port, with the pids mccboard launched.
//!
//! - **Reopened without mccboard:** a profile's main process that no launch by mccboard
//!   started (its pid, or a parent within a minute of the launch).
//! - **Closed by an update:** an open profile's main process vanished and, within a minute, a
//!   new main process of the default desktop profile appeared.
//! - A normal launch by mccboard, and a profile the user closes, stay silent.

mod common;

use std::path::{Path, PathBuf};

use common::{DESKTOP_EXE, TempHome, UUID_A, UUID_B, now, user_data_dir};
use jiff::{SignedDuration, Timestamp};
use mccboard_core::testing::{FIRST_FAKE_DESKTOP_PID, FakeEnvironment};
use mccboard_core::{
    AccountUuid, Core, DesktopNotice, DesktopNoticeKind, DesktopNotices, DesktopPackage, Language,
    OsKind, ProcessListing, RemoveLevel, TOOLTIP_MAX_UTF16,
};

/// The pid of mccboard itself, the parent of what it launches.
const MCCBOARD: u32 = 100;
/// Explorer: the parent of the default profile started from the Start menu.
const EXPLORER: u32 = 1;
/// The process an update restart comes from (not mccboard).
const UPDATER: u32 = 777;

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

/// A Windows home with the primary instance registered as A, `~/.claude-b` registered as B
/// with a desktop profile in `~/profiles/b`, and the app installed.
fn setup() -> (TempHome, Core, FakeEnvironment) {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    core.register(&b.dir).unwrap();
    core.add_desktop_profile(&uuid(UUID_B), Some(&folder_b(&home)))
        .unwrap();
    let install_dir = home
        .path()
        .join("WindowsApps")
        .join("Claude_1.0.2342.0_x64__pzs8sxrjxfjjc");
    std::fs::create_dir_all(install_dir.join("app")).unwrap();
    std::fs::write(install_dir.join("app").join("Claude.exe"), "MZ").unwrap();
    env.set_desktop_app_packages(vec![DesktopPackage {
        full_name: "Claude_1.0.2342.0_x64__pzs8sxrjxfjjc".into(),
        install_dir,
    }]);
    (home, core, env)
}

fn folder_b(home: &TempHome) -> PathBuf {
    home.path().join("profiles").join("b")
}

fn listing(pid: u32, parent: u32, args: &[String]) -> ProcessListing {
    ProcessListing {
        pid,
        parent_pid: Some(parent),
        name: "Claude.exe".into(),
        exe: Some(PathBuf::from(DESKTOP_EXE)),
        args: std::iter::once(DESKTOP_EXE.to_owned())
            .chain(args.iter().cloned())
            .collect(),
    }
}

/// The main process of the profile in `folder`, and one renderer under it.
fn profile(pid: u32, parent: u32, folder: &Path) -> Vec<ProcessListing> {
    vec![
        listing(pid, parent, &[user_data_dir(folder)]),
        listing(
            pid + 1,
            pid,
            &["--type=renderer".into(), user_data_dir(folder)],
        ),
    ]
}

/// The default profile's main process, and one renderer under it.
fn default_profile(pid: u32, parent: u32) -> Vec<ProcessListing> {
    vec![
        listing(pid, parent, &[]),
        listing(pid + 1, pid, &["--type=renderer".into()]),
    ]
}

/// A process list made of the given groups.
fn running(groups: &[Vec<ProcessListing>]) -> Vec<ProcessListing> {
    groups.concat()
}

fn at(seconds: i64) -> Timestamp {
    now() + SignedDuration::from_secs(seconds)
}

/// Sets the process list and the clock, then polls in English.
fn poll(core: &Core, env: &FakeEnvironment, seconds: i64, processes: Vec<ProcessListing>) {
    env.set_now(at(seconds));
    env.set_desktop_processes(processes);
    core.poll_desktop_profiles(Language::English).unwrap();
}

fn notices(core: &Core) -> DesktopNotices {
    core.desktop_notices(Language::English).unwrap()
}

fn only(notices: &DesktopNotices) -> &DesktopNotice {
    assert_eq!(notices.notices.len(), 1, "{notices:#?}");
    &notices.notices[0]
}

/// Opens B through mccboard and returns the pid the fake launch gave it.
fn open_b(core: &Core) -> u32 {
    core.open_desktop_profile(&uuid(UUID_B)).unwrap().pid
}

// ---------------------------------------------------------------------------------------------
// Launches by mccboard stay silent

#[test]
fn a_normal_launch_by_mccboard_stays_silent() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    assert_eq!(b, FIRST_FAKE_DESKTOP_PID);
    let list = running(&[
        default_profile(500, EXPLORER),
        profile(b, MCCBOARD, &folder_b(&home)),
    ]);
    for seconds in [0, 5, 10, 3600] {
        poll(&core, &env, seconds, list.clone());
        assert!(notices(&core).notices.is_empty());
    }
}

#[test]
fn the_launched_process_may_hand_over_to_a_child_at_once() {
    // The packaged app may start its real main process from the one mccboard launched.
    let (home, core, env) = setup();
    poll(&core, &env, 0, running(&[default_profile(500, EXPLORER)]));
    env.set_now(at(1));
    let launched = open_b(&core);
    // The launched process is gone already; its child runs the profile.
    poll(
        &core,
        &env,
        2,
        running(&[
            default_profile(500, EXPLORER),
            profile(4100, launched, &folder_b(&home)),
        ]),
    );
    poll(
        &core,
        &env,
        3600,
        running(&[
            default_profile(500, EXPLORER),
            profile(4100, launched, &folder_b(&home)),
        ]),
    );
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn the_default_profile_is_never_reported_as_reopened() {
    let (_home, core, env) = setup();
    poll(&core, &env, 0, running(&[default_profile(500, UPDATER)]));
    poll(&core, &env, 5, running(&[default_profile(600, UPDATER)]));
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn launches_are_remembered_across_a_restart_of_mccboard() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    assert!(
        home.path()
            .join(".mccboard/state/desktop-launches.json")
            .is_file()
    );
    // mccboard restarts: a new core in the same home, nothing in memory.
    let restarted = Core::new(home.path(), env.clone());
    env.set_desktop_processes(running(&[profile(b, MCCBOARD, &folder_b(&home))]));
    let first = restarted.poll_desktop_profiles(Language::English).unwrap();
    assert!(first.notices.is_empty(), "{first:#?}");
}

#[test]
fn without_profiles_nothing_is_watched() {
    let home = TempHome::new();
    let (core, env) = home.core();
    env.set_os(OsKind::Windows);
    env.set_desktop_processes(running(&[default_profile(500, UPDATER)]));
    let notices = core.poll_desktop_profiles(Language::English).unwrap();
    assert!(notices.notices.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Reopened without mccboard

#[test]
fn a_profile_the_app_restarted_on_its_own_is_noticed_until_reopened_by_mccboard() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    assert!(notices(&core).notices.is_empty());

    // The update restart: B comes back with its folder, started by something else.
    poll(
        &core,
        &env,
        300,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    let seen = notices(&core);
    let notice = only(&seen);
    assert_eq!(notice.kind, DesktopNoticeKind::ReopenedWithoutMccboard);
    assert_eq!(notice.account, uuid(UUID_B));
    assert_eq!(notice.label, "B");
    assert!(notice.stray_running);
    assert!(
        !notice.can_reopen,
        "opening it now would only bring the stray copy to the front"
    );
    assert_eq!(notice.since, at(300));
    assert!(notice.message.contains("B"), "{}", notice.message);
    assert!(notice.message.contains("Close B"), "{}", notice.message);
    assert_eq!(notice.reopen, "Reopen B");

    // Still there at the next poll, since unchanged.
    poll(
        &core,
        &env,
        305,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    assert_eq!(only(&notices(&core)).since, at(300));

    // The user closes the stray copy: now it can be reopened.
    poll(&core, &env, 400, Vec::new());
    let seen = notices(&core);
    let notice = only(&seen);
    assert_eq!(notice.kind, DesktopNoticeKind::ReopenedWithoutMccboard);
    assert!(!notice.stray_running);
    assert!(notice.can_reopen);

    // Reopened from mccboard: the notice goes.
    let again = open_b(&core);
    poll(
        &core,
        &env,
        402,
        running(&[profile(again, MCCBOARD, &folder_b(&home))]),
    );
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn a_child_of_the_launched_process_started_long_after_is_not_a_launch() {
    // The app relaunching itself from its old main process after an update.
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(
        &core,
        &env,
        4 * 3600,
        running(&[profile(4600, b, &folder_b(&home))]),
    );
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ReopenedWithoutMccboard
    );
}

#[test]
fn a_launched_pid_seen_gone_and_reused_is_not_mccboards_any_more() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 5, Vec::new());
    poll(
        &core,
        &env,
        10,
        running(&[profile(b, UPDATER, &folder_b(&home))]),
    );
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ReopenedWithoutMccboard
    );
}

#[test]
fn a_profile_started_without_mccboard_is_reported_at_mccboards_first_look() {
    // Nothing remembered launched it: started by hand, or by an update while mccboard was off.
    let (home, core, env) = setup();
    poll(
        &core,
        &env,
        0,
        running(&[profile(4500, EXPLORER, &folder_b(&home))]),
    );
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ReopenedWithoutMccboard
    );
}

// ---------------------------------------------------------------------------------------------
// Closed by an update

#[test]
fn a_profile_that_vanished_while_a_new_default_copy_appeared_was_closed_by_an_update() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[
            default_profile(500, EXPLORER),
            profile(b, MCCBOARD, &folder_b(&home)),
        ]),
    );
    // The update closes both and restarts the default copy only.
    poll(&core, &env, 5, running(&[default_profile(900, UPDATER)]));
    let seen = notices(&core);
    let notice = only(&seen);
    assert_eq!(notice.kind, DesktopNoticeKind::ClosedByUpdate);
    assert_eq!(notice.account, uuid(UUID_B));
    assert!(!notice.stray_running);
    assert!(notice.can_reopen);
    assert_eq!(notice.message, "B was closed by an update: reopen it.");
    assert_eq!(notice.reopen, "Reopen B");

    // Reopened by mccboard: gone.
    let again = open_b(&core);
    poll(
        &core,
        &env,
        20,
        running(&[
            default_profile(900, UPDATER),
            profile(again, MCCBOARD, &folder_b(&home)),
        ]),
    );
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn the_default_copy_may_appear_within_a_minute_after_the_profile_vanished() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 5, Vec::new());
    poll(&core, &env, 30, Vec::new());
    assert!(notices(&core).notices.is_empty());
    poll(&core, &env, 60, running(&[default_profile(900, UPDATER)]));
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ClosedByUpdate
    );
}

#[test]
fn a_profile_closed_by_hand_stays_silent() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    let default = default_profile(500, EXPLORER);
    poll(
        &core,
        &env,
        0,
        running(&[default.clone(), profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 5, default.clone());
    poll(&core, &env, 30, default.clone());
    poll(&core, &env, 90, default);
    // The default app started by hand more than a minute later.
    poll(&core, &env, 120, running(&[default_profile(900, EXPLORER)]));
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn the_default_profile_opened_by_mccboard_after_closing_one_stays_silent() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 5, Vec::new());
    let default = core.open_default_desktop_profile().unwrap().pid;
    poll(
        &core,
        &env,
        10,
        running(&[default_profile(default, MCCBOARD)]),
    );
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn a_profile_back_without_mccboard_after_vanishing_is_reported_as_reopened() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 5, running(&[default_profile(900, UPDATER)]));
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ClosedByUpdate
    );
    poll(
        &core,
        &env,
        10,
        running(&[
            default_profile(900, UPDATER),
            profile(4700, UPDATER, &folder_b(&home)),
        ]),
    );
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ReopenedWithoutMccboard
    );
}

// ---------------------------------------------------------------------------------------------
// Dismissing, languages, the tray tooltip

#[test]
fn a_dismissed_notice_stays_hidden_until_it_happens_again() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    poll(
        &core,
        &env,
        0,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(
        &core,
        &env,
        5,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    assert!(core.dismiss_desktop_notice(&uuid(UUID_B)));
    assert!(notices(&core).notices.is_empty());
    poll(
        &core,
        &env,
        10,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    poll(&core, &env, 15, Vec::new());
    assert!(notices(&core).notices.is_empty());
    assert!(!core.dismiss_desktop_notice(&uuid(UUID_B)));

    // Reopened properly, then restarted by the app again: shown again.
    let again = open_b(&core);
    poll(
        &core,
        &env,
        20,
        running(&[profile(again, MCCBOARD, &folder_b(&home))]),
    );
    poll(
        &core,
        &env,
        30,
        running(&[profile(4800, UPDATER, &folder_b(&home))]),
    );
    assert_eq!(
        only(&notices(&core)).kind,
        DesktopNoticeKind::ReopenedWithoutMccboard
    );
}

#[test]
fn a_removed_profile_loses_its_notice() {
    let (home, core, env) = setup();
    poll(
        &core,
        &env,
        0,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    assert_eq!(notices(&core).notices.len(), 1);
    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();
    poll(
        &core,
        &env,
        5,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    assert!(notices(&core).notices.is_empty());
}

#[test]
fn notices_read_in_italian() {
    let (home, core, env) = setup();
    poll(
        &core,
        &env,
        0,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    let seen = core.desktop_notices(Language::Italian).unwrap();
    let notice = only(&seen);
    assert_eq!(seen.language, Language::Italian);
    assert!(notice.message.contains("Chiudi B"), "{}", notice.message);
    assert_eq!(notice.reopen, "Riapri B");
    assert!(
        notice.tooltip.contains("senza mccboard"),
        "{}",
        notice.tooltip
    );

    poll(&core, &env, 5, Vec::new());
    poll(&core, &env, 6, running(&[default_profile(900, UPDATER)]));
    let b = open_b(&core);
    poll(
        &core,
        &env,
        10,
        running(&[profile(b, MCCBOARD, &folder_b(&home))]),
    );
    poll(&core, &env, 15, running(&[default_profile(901, UPDATER)]));
    let seen = core.desktop_notices(Language::Italian).unwrap();
    assert_eq!(
        only(&seen).message,
        "B è stato chiuso da un aggiornamento: riaprilo."
    );
}

#[test]
fn the_tray_tooltip_puts_notices_first_and_still_fits() {
    let (home, core, env) = setup();
    poll(
        &core,
        &env,
        0,
        running(&[profile(4500, UPDATER, &folder_b(&home))]),
    );
    let seen = notices(&core);
    let usage = "A · 5h 35% · 7d 12%\nB · 5h 80% · 7d 40%";
    let tooltip = seen.tray_tooltip(usage);
    assert!(tooltip.starts_with(&seen.notices[0].tooltip), "{tooltip}");
    assert!(tooltip.ends_with(usage), "{tooltip}");

    let long = "x".repeat(120);
    let tooltip = seen.tray_tooltip(&long);
    assert!(tooltip.encode_utf16().count() <= TOOLTIP_MAX_UTF16);
    assert!(tooltip.starts_with(&seen.notices[0].tooltip));

    let empty = core
        .desktop_notices(Language::English)
        .map(|mut n| {
            n.notices.clear();
            n
        })
        .unwrap();
    assert_eq!(empty.tray_tooltip(usage), usage);
}

// ---------------------------------------------------------------------------------------------
// The launches file

#[test]
fn an_unreadable_launches_file_keeps_the_launches_read_last() {
    let (home, core, env) = setup();
    let b = open_b(&core);
    let list = running(&[profile(b, MCCBOARD, &folder_b(&home))]);
    poll(&core, &env, 0, list.clone());
    let file = home.path().join(".mccboard/state/desktop-launches.json");
    std::fs::write(&file, "not json").unwrap();
    poll(&core, &env, 5, list);
    assert!(notices(&core).notices.is_empty());

    // The next opening starts the file over.
    let again = open_b(&core);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&format!("\"pid\": {again}")), "{text}");
}

#[test]
fn the_launches_file_keeps_only_running_or_recent_launches() {
    let (home, core, env) = setup();
    let first = open_b(&core);
    env.set_now(at(3600));
    env.set_desktop_processes(running(&[default_profile(500, EXPLORER)]));
    let second = open_b(&core);
    let text =
        std::fs::read_to_string(home.path().join(".mccboard/state/desktop-launches.json")).unwrap();
    assert!(!text.contains(&format!("\"pid\": {first}")), "{text}");
    assert!(text.contains(&format!("\"pid\": {second}")), "{text}");
}
