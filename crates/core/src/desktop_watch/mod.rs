//! Noticing a desktop profile the app reopened on its own (spec #34, ticket #40, ADR 0008).
//!
//! The app updates itself and restarts. The restart loses `CLAUDE_CONFIG_DIR`, and may lose
//! `--user-data-dir` too. A watch fed successive process lists (the app polls every few
//! seconds) tells two cases apart, and never stops a process:
//! - **reopened without mccboard:** a stored profile's main process that no launch by mccboard
//!   started. mccboard started it when its pid is one mccboard launched, or when one of its
//!   ancestors among the listed processes is, and it appeared within [`HANDOVER`] of that
//!   launch (the packaged app may start its real main process from the one launched). A
//!   process the old main process starts hours later, as an app relaunching itself after an
//!   update does, is not a launch by mccboard;
//! - **closed by an update:** a profile's main process vanished and, within [`PAIRING`]
//!   before or after, a new main process of the default desktop profile appeared that
//!   mccboard didn't launch: the restart came back without the profile's data folder.
//!
//! A profile the user closes, and every launch by mccboard, stay silent.
//!
//! **Launches** are remembered in `~/.mccboard/state/` ([`launches`]), so launches by the CLI
//! and by an earlier run of the app count too. A launch whose process the watch saw gone
//! (neither listed nor the parent of a listed process) is over: its pid, reused later, is not
//! mccboard's any more.
//!
//! **mccboard starting.** At the first poll nothing vanished or appeared. A profile running
//! then counts as launched by mccboard only by the remembered launches; for its children, the
//! time they appeared isn't known, so the handover window is not checked. A profile running
//! with no remembered launch behind it is reported: started by hand or by an update while
//! mccboard was off, its Code tab may lack its instance either way.
//!
//! **Notices** stay until the profile runs again from a launch by mccboard, or the user
//! dismisses them. A dismissed notice stays hidden until it happens again.

pub(crate) mod launches;
mod notice;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};

use crate::config::{Config, ConfigFile};
use crate::desktop_process::{self, DesktopProcess};
use crate::{AccountUuid, Environment, Error, Language, Paths};
use launches::Launch;
pub use notice::{DesktopNotice, DesktopNoticeKind, DesktopNotices};

/// How long after a launch its child may appear and still be the launch's main process.
const HANDOVER: SignedDuration = SignedDuration::from_secs(60);

/// How close a profile vanishing and a new default copy appearing must be to pair up.
const PAIRING: SignedDuration = SignedDuration::from_secs(60);

/// How far up the parent chain a launch is looked for.
const MAX_ANCESTORS: usize = 8;

/// What the watch remembers between polls.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    /// A process list was seen already.
    started: bool,
    /// The launches as last read.
    launches: Vec<Launch>,
    /// Launches whose process was seen gone, by pid and time.
    over: BTreeSet<(u32, Timestamp)>,
    /// When each running main process was first seen; `None`: at the first poll.
    first_seen: HashMap<u32, Option<Timestamp>>,
    /// The profiles with a main process at the last poll.
    open: BTreeSet<AccountUuid>,
    /// The default profile's main processes at the last poll.
    default_mains: BTreeSet<u32>,
    /// Profiles whose main process vanished lately, and when.
    vanished: BTreeMap<AccountUuid, Timestamp>,
    /// When a new default copy last appeared.
    default_appeared: Option<Timestamp>,
    notices: BTreeMap<AccountUuid, NoticeState>,
}

/// A notice as the watch keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoticeState {
    pub kind: DesktopNoticeKind,
    /// The copy reopened without mccboard still runs.
    pub stray_running: bool,
    pub since: Timestamp,
    pub dismissed: bool,
}

/// A stored profile, as the watch needs it.
pub(crate) struct Watched {
    pub account: AccountUuid,
    pub data_folder: PathBuf,
}

impl Watch {
    /// Takes in the process list `processes` seen at `now`, with the launches remembered
    /// (`None`: they couldn't be read; the last ones read stand).
    pub fn observe(
        &mut self,
        now: Timestamp,
        home: &Path,
        profiles: &[Watched],
        processes: &[DesktopProcess],
        launches: Option<Vec<Launch>>,
    ) {
        if let Some(launches) = launches {
            self.launches = launches;
        }
        let parents: HashMap<u32, Option<u32>> =
            processes.iter().map(|p| (p.pid, p.parent_pid)).collect();
        for launch in &self.launches {
            let alive = parents.contains_key(&launch.pid)
                || processes.iter().any(|p| p.parent_pid == Some(launch.pid));
            if !alive {
                self.over.insert((launch.pid, launch.at));
            }
        }

        let mains: Vec<DesktopProcess> =
            processes.iter().filter(|p| p.is_main()).cloned().collect();
        let first = !self.started;
        self.first_seen
            .retain(|pid, _| mains.iter().any(|p| p.pid == *pid));
        for main in &mains {
            self.first_seen
                .entry(main.pid)
                .or_insert(if first { None } else { Some(now) });
        }

        // Profiles no longer stored lose what the watch knew of them.
        let stored = |account: &AccountUuid| profiles.iter().any(|p| &p.account == account);
        self.notices.retain(|account, _| stored(account));
        self.vanished.retain(|account, _| stored(account));

        let mut open = BTreeSet::new();
        for profile in profiles {
            let own = desktop_process::using_folder(&mains, home, &profile.data_folder);
            let account = &profile.account;
            let stray = own
                .iter()
                .any(|main| !self.launched_by_mccboard(main, &parents));
            if stray {
                match self.notices.get_mut(account) {
                    Some(notice) if notice.kind == DesktopNoticeKind::ReopenedWithoutMccboard => {
                        notice.stray_running = true;
                    }
                    _ => {
                        self.notices.insert(
                            account.clone(),
                            NoticeState {
                                kind: DesktopNoticeKind::ReopenedWithoutMccboard,
                                stray_running: true,
                                since: now,
                                dismissed: false,
                            },
                        );
                    }
                }
            } else if !own.is_empty() {
                self.notices.remove(account);
            } else if let Some(notice) = self.notices.get_mut(account) {
                notice.stray_running = false;
            }
            if own.is_empty() {
                if self.open.contains(account) {
                    self.vanished.insert(account.clone(), now);
                }
            } else {
                open.insert(account.clone());
                self.vanished.remove(account);
            }
        }

        let default_mains: BTreeSet<u32> = desktop_process::running_default(&mains, home)
            .iter()
            .map(|p| p.pid)
            .collect();
        let appeared = default_mains.iter().any(|pid| {
            !self.default_mains.contains(pid) && {
                let main = mains.iter().find(|p| p.pid == *pid).expect("listed");
                !self.launched_by_mccboard(main, &parents)
            }
        });
        if appeared && !first {
            self.default_appeared = Some(now);
        }

        if let Some(appeared_at) = self.default_appeared {
            let paired: Vec<AccountUuid> = self
                .vanished
                .iter()
                .filter(|(_, vanished_at)| {
                    appeared_at.duration_since(**vanished_at).abs() <= PAIRING
                })
                .map(|(account, _)| account.clone())
                .collect();
            for account in paired {
                self.vanished.remove(&account);
                let reopened = self
                    .notices
                    .get(&account)
                    .is_some_and(|n| n.kind == DesktopNoticeKind::ReopenedWithoutMccboard);
                if !reopened {
                    self.notices.insert(
                        account,
                        NoticeState {
                            kind: DesktopNoticeKind::ClosedByUpdate,
                            stray_running: false,
                            since: now,
                            dismissed: false,
                        },
                    );
                }
            }
        }
        self.vanished
            .retain(|_, vanished_at| now.duration_since(*vanished_at) <= PAIRING);
        if self
            .default_appeared
            .is_some_and(|at| now.duration_since(at) > PAIRING)
        {
            self.default_appeared = None;
        }

        self.open = open;
        self.default_mains = default_mains;
        self.started = true;
    }

    /// Whether mccboard started `main`: its pid is a launch, or an ancestor is and `main`
    /// appeared within [`HANDOVER`] of it.
    fn launched_by_mccboard(
        &self,
        main: &DesktopProcess,
        parents: &HashMap<u32, Option<u32>>,
    ) -> bool {
        let live = |pid: u32| {
            self.launches
                .iter()
                .find(|l| l.pid == pid && !self.over.contains(&(l.pid, l.at)))
        };
        if live(main.pid).is_some() {
            return true;
        }
        let appeared = self.first_seen.get(&main.pid).copied().flatten();
        let mut ancestor = main.parent_pid;
        for _ in 0..MAX_ANCESTORS {
            let Some(pid) = ancestor else {
                break;
            };
            if let Some(launch) = live(pid) {
                return appeared.is_none_or(|at| at.duration_since(launch.at) <= HANDOVER);
            }
            ancestor = parents.get(&pid).copied().flatten();
        }
        false
    }

    /// Hides `account`'s notice; `false` when it has none showing.
    pub fn dismiss(&mut self, account: &AccountUuid) -> bool {
        match self.notices.get_mut(account) {
            Some(notice) if !notice.dismissed => {
                notice.dismissed = true;
                true
            }
            _ => false,
        }
    }

    /// The notices showing, by account.
    pub fn showing(&self) -> impl Iterator<Item = (&AccountUuid, &NoticeState)> {
        self.notices.iter().filter(|(_, n)| !n.dismissed)
    }
}

fn lock(watch: &Mutex<Watch>) -> std::sync::MutexGuard<'_, Watch> {
    watch.lock().unwrap_or_else(|p| p.into_inner())
}

/// Takes a process list, feeds it to the watch, and returns the notices. Without stored
/// profiles nothing is listed and the watch starts over.
pub(crate) fn poll(
    home: &Path,
    paths: &Paths,
    env: &dyn Environment,
    watch: &Mutex<Watch>,
    language: Language,
) -> Result<DesktopNotices, Error> {
    let config = ConfigFile::load(&paths.config_file())?.config;
    let mut watch = lock(watch);
    if config.desktop_profiles.is_empty() {
        *watch = Watch::default();
    } else {
        let processes = desktop_process::desktop_processes(env.desktop_processes());
        let profiles: Vec<Watched> = config
            .desktop_profiles
            .iter()
            .map(|p| Watched {
                account: p.account.clone(),
                data_folder: p.data_folder.clone(),
            })
            .collect();
        watch.observe(
            env.now(),
            home,
            &profiles,
            &processes,
            launches::load(paths),
        );
    }
    Ok(notice::view(&config, &watch, language))
}

/// The notices as of the last poll.
pub(crate) fn notices(
    paths: &Paths,
    watch: &Mutex<Watch>,
    language: Language,
) -> Result<DesktopNotices, Error> {
    let config: Config = ConfigFile::load(&paths.config_file())?.config;
    Ok(notice::view(&config, &lock(watch), language))
}

pub(crate) fn dismiss(watch: &Mutex<Watch>, account: &AccountUuid) -> bool {
    lock(watch).dismiss(account)
}
