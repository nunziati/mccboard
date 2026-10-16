//! The tray view model (stories 60–64): everything the tray shows, computed here so the app
//! only renders it.
//!
//! - **Icon:** the percentage of the account closest to a limit, the higher of its session and
//!   weekly windows, in that account's color. Model windows don't count. A tie goes to the
//!   first account in label order, so the icon doesn't flicker between equal accounts.
//! - **Tooltip:** every account with its windows and reset times, compact enough for the
//!   Windows tray (127 UTF-16 units): detail is dropped step by step until it fits, but every
//!   account stays named. A single account reads as a plain usage indicator, without label.
//!   When Claude Code renewed an expired token, or failed to, a short note says when (or when
//!   it tries again); it is the first detail dropped.
//! - **Menu status lines:** the full detail, one account at a time, for platforms without
//!   tooltips (Linux) and for what the tooltip had to drop.
//!
//! Texts come in the app's [`Language`], with reset times relative to now and in the given
//! time zone.

pub(crate) mod profiles;
mod text;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::usage::as_of;
use crate::{
    Account, AccountUsage, AccountUuid, Color, Error, Language, TokenRenewal, UsageReport,
    UsageStatus,
};
use text::{Status, Texts};

/// The Windows tray keeps at most this many UTF-16 units of a tooltip.
pub const TOOLTIP_MAX_UTF16: usize = 127;

/// Everything the tray shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayView {
    /// The language of every text below.
    pub language: Language,
    pub icon: TrayIcon,
    /// At most [`TOOLTIP_MAX_UTF16`] UTF-16 units, lines separated by `\n`.
    pub tooltip: String,
    /// A message shown in the menu instead of accounts: no instance registered yet, or why
    /// usage can't be shown.
    pub notice: Option<String>,
    /// Every registered account, in label order, with its status lines for the menu.
    pub accounts: Vec<TrayAccount>,
    /// No instance is registered: the tray offers to open Instances to set one up.
    pub no_instances: bool,
    pub menu: TrayMenu,
}

/// What the tray icon draws.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayIcon {
    /// The worst account's highest session or weekly window, 0–100; `None` when no account
    /// has a reading.
    pub percent: Option<u8>,
    /// The worst account's color; a single account's color, or a neutral gray, when no
    /// account has a reading.
    pub color: Color,
    /// The worst account, when there is one.
    pub account: Option<AccountUuid>,
    /// The percentage comes from a stale reading.
    pub stale: bool,
    /// The worst account's access token is expired; without a worst account, any account's
    /// is.
    pub token_expired: bool,
}

/// One account as the tray shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayAccount {
    pub account: Account,
    /// Where its usage stands, as the usage report says.
    pub status: UsageStatus,
    /// When its last reading was read, if it has one.
    pub read_at: Option<Timestamp>,
    /// The higher of its session and weekly windows, when it has a reading.
    pub percent: Option<u8>,
    /// This account drives the icon.
    pub worst: bool,
    pub stale: bool,
    pub token_expired: bool,
    /// Session, weekly, then model windows, as last read; past resets read as zero.
    pub windows: Vec<TrayWindow>,
    /// Claude Code renewing the account's expired token, as the usage report says.
    pub renewal: Option<TokenRenewal>,
    /// The menu heading: label and percentage, or why there is none ("A · 35%").
    pub title: String,
    /// The menu lines under the heading: every window with its reset, then when it was read
    /// if the reading is stale, then the token's renewal.
    pub lines: Vec<String>,
}

/// A usage window, rounded for display.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayWindow {
    pub name: WindowName,
    pub percent: u8,
    pub resets_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowName {
    Session,
    Weekly,
    /// A model window, named by the endpoint.
    Model {
        model: String,
    },
}

/// The menu's labels.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayMenu {
    pub open_instances: String,
    pub refresh: String,
    pub settings: String,
    pub language: String,
    pub languages: Vec<LanguageChoice>,
    /// The settings item that lets Claude Code renew an expired token (#30).
    pub renew_tokens: String,
    /// Whether that preference is on, so the item shows checked.
    pub renew_tokens_on: bool,
    pub quit: String,
}

/// A language in the settings menu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LanguageChoice {
    pub language: Language,
    /// The language's own name ("Italiano").
    pub name: String,
    /// The language in use.
    pub selected: bool,
}

impl TrayView {
    /// The tray when usage can't be shown at all (for example a corrupt `config.toml`): no
    /// percentage, and the reason in the tooltip and the menu.
    pub fn unavailable(error: &Error, language: Language) -> TrayView {
        let texts = Texts::new(language, TimeZone::UTC, Timestamp::UNIX_EPOCH);
        let message = texts.unavailable(error.code());
        TrayView {
            language,
            icon: TrayIcon {
                percent: None,
                color: neutral(),
                account: None,
                stale: false,
                token_expired: false,
            },
            tooltip: fit(vec![format!("mccboard: {message}")]),
            notice: Some(message),
            accounts: Vec::new(),
            no_instances: false,
            // The preference can't be read either; the item shows as the default.
            menu: texts.menu(true),
        }
    }
}

/// Computes the tray view from a usage report, as of `now`. `no_instances` is whether no
/// instance is registered.
pub(crate) fn view(
    report: &UsageReport,
    no_instances: bool,
    renew_tokens_on: bool,
    language: Language,
    tz: &TimeZone,
    now: Timestamp,
) -> TrayView {
    let texts = Texts::new(language, tz.clone(), now);
    let mut accounts: Vec<TrayAccount> = report
        .accounts
        .iter()
        .map(|usage| account(usage, now, &texts))
        .collect();

    // The highest percentage wins; on a tie the first in label order (`max_by` keeps the last).
    let worst = accounts
        .iter()
        .enumerate()
        .filter_map(|(index, a)| Some((index, peak(&report.accounts[index], now)?, a)))
        .rev()
        .max_by(|(_, x, _), (_, y, _)| x.total_cmp(y))
        .map(|(index, _, _)| index);
    if let Some(index) = worst {
        accounts[index].worst = true;
    }

    let icon = match worst {
        Some(index) => {
            let a = &accounts[index];
            TrayIcon {
                percent: a.percent,
                color: a.account.color.clone(),
                account: Some(a.account.uuid.clone()),
                stale: a.stale,
                token_expired: a.token_expired,
            }
        }
        None => TrayIcon {
            percent: None,
            color: match accounts.as_slice() {
                [single] if !no_instances => single.account.color.clone(),
                _ => neutral(),
            },
            account: None,
            stale: false,
            token_expired: accounts.iter().any(|a| a.token_expired),
        },
    };

    let (tooltip, notice) = if no_instances {
        let message = texts.no_instances();
        (fit(vec![format!("mccboard: {message}")]), Some(message))
    } else {
        (tooltip(&accounts, &texts), None)
    };

    TrayView {
        language,
        icon,
        tooltip,
        notice,
        accounts: if no_instances { Vec::new() } else { accounts },
        no_instances,
        menu: texts.menu(renew_tokens_on),
    }
}

fn neutral() -> Color {
    crate::account::neutral_color()
}

fn percent(utilization: f64) -> u8 {
    utilization.clamp(0.0, 100.0).round() as u8
}

/// The higher of an account's session and weekly utilization as of `now`, unrounded.
fn peak(usage: &AccountUsage, now: Timestamp) -> Option<f64> {
    let reading = as_of(usage.reading.clone()?, now);
    [reading.session, reading.weekly]
        .into_iter()
        .flatten()
        .map(|w| w.utilization)
        .reduce(f64::max)
}

fn account(usage: &AccountUsage, now: Timestamp, texts: &Texts) -> TrayAccount {
    let reading = usage.reading.clone().map(|r| as_of(r, now));
    let token_expired = usage.status == UsageStatus::TokenExpired;
    let mut windows = Vec::new();
    if let Some(reading) = &reading {
        let main = [
            (WindowName::Session, reading.session),
            (WindowName::Weekly, reading.weekly),
        ];
        for (name, window) in main {
            if let Some(window) = window {
                windows.push(TrayWindow {
                    name,
                    percent: percent(window.utilization),
                    resets_at: window.resets_at,
                });
            }
        }
        for model in &reading.models {
            windows.push(TrayWindow {
                name: WindowName::Model {
                    model: model.model.clone(),
                },
                percent: percent(model.utilization),
                resets_at: model.resets_at,
            });
        }
    }
    let mut account = TrayAccount {
        account: usage.account.clone(),
        status: usage.status.clone(),
        read_at: reading.as_ref().map(|r| r.read_at),
        percent: peak(usage, now).map(percent),
        worst: false,
        stale: usage.stale,
        token_expired,
        windows,
        renewal: usage.renewal.clone(),
        title: String::new(),
        lines: Vec::new(),
    };
    account.title = texts.title(&account.account.label, account.percent, status_of(&account));
    account.lines = account
        .windows
        .iter()
        .map(|w| texts.window_line(w))
        .collect();
    if let Some(read_at) = account.read_at.filter(|_| account.stale) {
        account.lines.push(texts.read_at(read_at));
    }
    if let Some(line) = texts.renewal_line(&account) {
        account.lines.push(line);
    }
    account
}

/// What the tray says about an account besides its numbers.
fn status_of(account: &TrayAccount) -> Status {
    if account.read_at.is_some() {
        return if account.token_expired {
            Status::TokenExpired
        } else if account.stale {
            Status::Stale
        } else {
            Status::Current
        };
    }
    match &account.status {
        UsageStatus::Ok | UsageStatus::NeverRead => Status::NeverRead,
        UsageStatus::TokenExpired => Status::TokenExpired,
        UsageStatus::NotLoggedIn => Status::NotLoggedIn,
        UsageStatus::CredentialsUnreadable => Status::CredentialsUnreadable,
        UsageStatus::Failed { .. } => Status::Failed,
    }
}

/// How much a tooltip says, from everything to the bare minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Detail {
    /// Every window with its reset, and the token's renewal.
    WithRenewal,
    /// Every window with its reset.
    Full,
    /// Model windows without their reset.
    ModelsWithoutResets,
    /// No model windows.
    NoModels,
    /// Session and weekly windows, no resets.
    NoResets,
    /// One percentage per account.
    Peak,
}

const DETAILS: [Detail; 6] = [
    Detail::WithRenewal,
    Detail::Full,
    Detail::ModelsWithoutResets,
    Detail::NoModels,
    Detail::NoResets,
    Detail::Peak,
];

/// The most detailed tooltip that fits the Windows limit.
fn tooltip(accounts: &[TrayAccount], texts: &Texts) -> String {
    let single = accounts.len() == 1;
    let mut lines = Vec::new();
    for detail in DETAILS {
        lines = if single {
            texts.single_tooltip(&accounts[0], detail)
        } else {
            accounts
                .iter()
                .map(|a| texts.account_tooltip(a, detail))
                .collect()
        };
        if fits(&lines.join("\n")) {
            break;
        }
    }
    fit(lines)
}

fn fits(text: &str) -> bool {
    text.encode_utf16().count() <= TOOLTIP_MAX_UTF16
}

/// Joins `lines`, keeping whole lines while they fit and ending with "…" if some didn't.
fn fit(lines: Vec<String>) -> String {
    let all = lines.join("\n");
    if fits(&all) {
        return all;
    }
    let mut out = String::new();
    for line in lines {
        let next = if out.is_empty() {
            line.clone()
        } else {
            format!("{out}\n{line}")
        };
        if !fits(&format!("{next}\n…")) {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        // Not even one line fits: cut it.
        let mut cut = String::new();
        for c in all.chars() {
            if !fits(&format!("{cut}{c}…")) {
                break;
            }
            cut.push(c);
        }
        return format!("{cut}…");
    }
    format!("{out}\n…")
}
