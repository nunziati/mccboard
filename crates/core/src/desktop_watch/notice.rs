//! The notices as the front ends show them: in the tray (a tooltip line and a "Reopen B" menu
//! item) and in the window (a banner with the message and a Reopen button). Texts come in the
//! app's language; every one is matched for every language, so a missing translation doesn't
//! compile.

use jiff::Timestamp;
use serde::Serialize;

use super::{NoticeState, Watch};
use crate::config::Config;
use crate::{AccountUuid, Color, Language, TOOLTIP_MAX_UTF16};

/// The color of a profile whose account isn't in `config.toml` (a hand-edited entry).
const NEUTRAL_COLOR: &str = "#6b7280";

/// Every desktop profile notice showing, in one language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopNotices {
    pub language: Language,
    /// By label.
    pub notices: Vec<DesktopNotice>,
}

/// A desktop profile the app reopened, or closed, on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopNotice {
    /// The profile's account.
    pub account: AccountUuid,
    /// The account's label; its UUID when the account isn't registered.
    pub label: String,
    pub color: Color,
    pub kind: DesktopNoticeKind,
    /// The copy reopened without mccboard still runs: it has to be closed before the profile
    /// can be opened the right way (opening it now would only bring that copy to the front).
    pub stray_running: bool,
    /// When it was first noticed.
    pub since: Timestamp,
    /// What happened and what to do, in a sentence.
    pub message: String,
    /// A short line for the tray tooltip.
    pub tooltip: String,
    /// The action's label: "Reopen B". It opens the profile as mccboard always does and
    /// quits nothing.
    pub reopen: String,
    /// Whether the action can help now: not while the stray copy runs.
    pub can_reopen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNoticeKind {
    /// The profile runs, or ran, from a process mccboard didn't start, probably the app's
    /// restart after an update: its Code tab may lack its instance (`CLAUDE_CONFIG_DIR`).
    ReopenedWithoutMccboard,
    /// The profile's processes vanished while a new copy of the default profile appeared: an
    /// update closed it and restarted the app without its data folder.
    ClosedByUpdate,
}

impl DesktopNotices {
    /// The tray tooltip: a line per notice, then the lines of `tooltip` (the usage tooltip)
    /// that still fit in [`TOOLTIP_MAX_UTF16`] units.
    pub fn tray_tooltip(&self, tooltip: &str) -> String {
        if self.notices.is_empty() {
            return tooltip.to_owned();
        }
        let notices = self.notices.iter().map(|n| (n.tooltip.as_str(), false));
        let usage = tooltip.lines().map(|line| (line, true));
        let mut lines: Vec<&str> = Vec::new();
        let mut used = 0;
        for (line, from_usage) in notices.chain(usage) {
            let cost = line.encode_utf16().count() + usize::from(!lines.is_empty());
            if used + cost > TOOLTIP_MAX_UTF16 {
                if from_usage {
                    break;
                }
                continue;
            }
            used += cost;
            lines.push(line);
        }
        lines.join("\n")
    }
}

pub(super) fn view(config: &Config, watch: &Watch, language: Language) -> DesktopNotices {
    let mut notices: Vec<DesktopNotice> = watch
        .showing()
        .map(|(account, state)| {
            let registered = config.account(account);
            let label = registered.map_or_else(|| account.to_string(), |a| a.label.clone());
            let color = registered.map_or_else(
                || NEUTRAL_COLOR.parse().expect("a valid color"),
                |a| a.color.clone(),
            );
            notice(language, account, label, color, state)
        })
        .collect();
    notices.sort_by(|a, b| {
        (a.label.to_lowercase(), &a.account).cmp(&(b.label.to_lowercase(), &b.account))
    });
    DesktopNotices { language, notices }
}

fn notice(
    language: Language,
    account: &AccountUuid,
    label: String,
    color: Color,
    state: &NoticeState,
) -> DesktopNotice {
    let it = language == Language::Italian;
    let l = &label;
    let (message, tooltip) = match (state.kind, state.stray_running, it) {
        (DesktopNoticeKind::ReopenedWithoutMccboard, true, false) => (
            format!(
                "Claude reopened {l} without mccboard, probably after an update: its Code tab \
                 may be using ~/.claude. Close {l}, then reopen it from mccboard."
            ),
            format!("{l}: reopened without mccboard"),
        ),
        (DesktopNoticeKind::ReopenedWithoutMccboard, true, true) => (
            format!(
                "Claude ha riaperto {l} senza mccboard, probabilmente dopo un aggiornamento: la \
                 sua scheda Code potrebbe usare ~/.claude. Chiudi {l}, poi riaprilo da mccboard."
            ),
            format!("{l}: riaperto senza mccboard"),
        ),
        (DesktopNoticeKind::ReopenedWithoutMccboard, false, false) => (
            format!("{l} was reopened without mccboard and is closed now: reopen it."),
            format!("{l}: closed, reopen it"),
        ),
        (DesktopNoticeKind::ReopenedWithoutMccboard, false, true) => (
            format!("{l} era stato riaperto senza mccboard e ora è chiuso: riaprilo."),
            format!("{l}: chiuso, riaprilo"),
        ),
        (DesktopNoticeKind::ClosedByUpdate, _, false) => (
            format!("{l} was closed by an update: reopen it."),
            format!("{l}: closed by an update"),
        ),
        (DesktopNoticeKind::ClosedByUpdate, _, true) => (
            format!("{l} è stato chiuso da un aggiornamento: riaprilo."),
            format!("{l}: chiuso da un aggiornamento"),
        ),
    };
    let reopen = if it {
        format!("Riapri {l}")
    } else {
        format!("Reopen {l}")
    };
    DesktopNotice {
        account: account.clone(),
        color,
        kind: state.kind,
        stray_running: state.stray_running,
        since: state.since,
        message,
        tooltip,
        reopen,
        can_reopen: !state.stray_running,
        label,
    }
}
