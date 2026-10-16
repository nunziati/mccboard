//! The taskbar widget's view model (#45, ADR 0009): everything the strip on the Windows taskbar
//! shows, computed here, on every OS, so the Windows module only docks, draws and forwards
//! clicks.
//!
//! - **Columns**: one per account, in `config.toml` order. Each has two rows, the session
//!   window on top and the weekly window below, and each row a bar and a time to reset.
//! - **Bar**: [`SEGMENTS`] segments, one filled for each 10% of utilization (at least one above
//!   0%), and the pace: how much of the window's time has passed. No pace for an idle window
//!   (no reset time: a session window nobody used for 5 hours).
//! - **Time to reset**: days, hours and minutes without leading zeros, leading units that are 0
//!   dropped (`5h 3m`, `42m`) and later ones kept (`3d 0h 7m`); seconds in the last minute
//!   (`42s`). An idle window shows a dash.
//! - **States**, as in the tray: a stale reading fades its column; an expired token without a
//!   reading shows dots in place of the rows, waiting for Claude Code to renew it; no reading at
//!   all shows a dash.
//! - **Colors**: each account's color, adjusted so it reads on both the light and the dark
//!   taskbar, and the taskbar's own text, dim and track colors ([`TaskbarTheme::palette`]).
//! - **Tooltip**: every account's windows with their resets as clock times, the model
//!   windows, and the age of a stale reading.
//! - **Redraw**: [`TaskbarWidget::redraw_in`] says when a text changes next: every second
//!   while a time to reset is in its last minute, else at the next minute.

mod text;

use std::time::Duration;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use crate::usage::as_of;
use crate::{AccountUsage, AccountUuid, Color, Language, UsageReport, UsageStatus, UsageWindow};
use text::Texts;

/// The segments of a bar: one for each 10% of utilization.
pub const SEGMENTS: u8 = 10;

/// The session window's length.
const SESSION_LENGTH: SignedDuration = SignedDuration::from_hours(5);

/// The weekly window's length.
const WEEKLY_LENGTH: SignedDuration = SignedDuration::from_hours(7 * 24);

/// The longest wait between two redraws: the minutes of a time to reset change at most this
/// often.
const REDRAW_AT_MOST_EVERY: Duration = Duration::from_secs(60);

/// Everything the taskbar widget shows, as of one moment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskbarWidget {
    /// The language of the tooltip.
    pub language: Language,
    /// The preference "Show the widget on the taskbar".
    pub enabled: bool,
    /// The preference "Show account labels": each column starts with its account's label.
    pub account_labels: bool,
    /// The labels of the rows, shown once at the left: the session window's, then the weekly
    /// window's.
    pub row_labels: [String; 2],
    /// One column per account, in `config.toml` order.
    pub columns: Vec<WidgetColumn>,
    /// The hover tooltip, one line per account.
    pub tooltip: String,
    /// When the widget must be computed again for a text to change.
    pub redraw_in: Duration,
}

impl TaskbarWidget {
    /// Whether there is something to show: the preference is on and an account is registered.
    pub fn visible(&self) -> bool {
        self.enabled && !self.columns.is_empty()
    }
}

/// One account's column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WidgetColumn {
    pub account: AccountUuid,
    /// The account's label, shown before the column when [`TaskbarWidget::account_labels`].
    pub label: String,
    /// The account's color as drawn on a light taskbar.
    pub light: AccountInk,
    /// The account's color as drawn on a dark taskbar.
    pub dark: AccountInk,
    pub state: ColumnState,
    /// The session window's row.
    pub session: WidgetRow,
    /// The weekly window's row.
    pub weekly: WidgetRow,
}

impl WidgetColumn {
    /// The account's colors on a taskbar of `theme`.
    pub fn ink(&self, theme: TaskbarTheme) -> &AccountInk {
        match theme {
            TaskbarTheme::Light => &self.light,
            TaskbarTheme::Dark => &self.dark,
        }
    }
}

/// An account's color on one taskbar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountInk {
    /// The bar's outline and filled segments.
    pub bar: Color,
    /// The account's label.
    pub text: Color,
}

/// Where an account's column stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnState {
    /// A current reading.
    Current,
    /// A stale reading: the column is drawn faded.
    Stale,
    /// No reading, and the token is expired: dots in place of the rows until Claude Code
    /// renews it.
    TokenRenewal,
    /// No reading: a dash in place of the rows.
    NoReading,
}

impl ColumnState {
    /// What each row shows instead of its bar and time to reset, if anything.
    pub fn placeholder(self) -> Option<&'static str> {
        match self {
            ColumnState::Current | ColumnState::Stale => None,
            ColumnState::TokenRenewal => Some("···"),
            ColumnState::NoReading => Some("—"),
        }
    }

    /// Whether the column is drawn faded.
    pub fn faded(self) -> bool {
        self == ColumnState::Stale
    }
}

/// One usage window in a column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WidgetRow {
    /// The utilization, 0–100.
    pub percent: u8,
    /// The filled segments, 0 to [`SEGMENTS`].
    pub segments: u8,
    /// How much of the window's time has passed, 0–1; `None` when the window is idle.
    pub pace: Option<f64>,
    pub reset: TimeToReset,
}

/// How long until a window resets, as the widget writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimeToReset {
    /// The numbers with their units, largest first; empty when the window is idle.
    pub parts: Vec<ResetPart>,
}

impl TimeToReset {
    /// What an idle window shows.
    pub const IDLE: &'static str = "—";

    /// The plain text: `3d 4h 7m`, `42s`, or a dash when idle.
    pub fn text(&self) -> String {
        if self.parts.is_empty() {
            return Self::IDLE.to_owned();
        }
        self.parts
            .iter()
            .map(|p| format!("{}{}", p.value, p.unit))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The time to reset of a window that resets at `at`, as of `now`; idle without a reset.
    fn until(at: Option<Timestamp>, now: Timestamp) -> TimeToReset {
        let Some(at) = at else {
            return TimeToReset { parts: Vec::new() };
        };
        let seconds = at.duration_since(now).as_secs().max(0) as u64;
        if seconds < 60 {
            return TimeToReset {
                parts: vec![ResetPart::new(seconds.max(1), 's')],
            };
        }
        let minutes = seconds / 60;
        let mut parts = vec![
            ResetPart::new(minutes / 1440, 'd'),
            ResetPart::new(minutes % 1440 / 60, 'h'),
            ResetPart::new(minutes % 60, 'm'),
        ];
        // Leading units that are 0 go; a 0 further right stays.
        while parts.len() > 1 && parts[0].value == 0 {
            parts.remove(0);
        }
        TimeToReset { parts }
    }
}

/// A number and its unit in a time to reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResetPart {
    pub value: u64,
    /// `d`, `h`, `m` or `s`.
    pub unit: char,
    /// The digits the number's slot is wide, so units line up from row to row: 2 for hours,
    /// minutes and seconds, 1 for days (a week has fewer than 10).
    pub slot: u8,
}

impl ResetPart {
    fn new(value: u64, unit: char) -> ResetPart {
        ResetPart {
            value,
            unit,
            slot: if unit == 'd' { 1 } else { 2 },
        }
    }
}

/// The taskbar's theme, as Windows sets it (`SystemUsesLightTheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskbarTheme {
    Light,
    Dark,
}

/// The taskbar's own colors, RGBA (not premultiplied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WidgetPalette {
    /// The time to reset and the pace mark, like the taskbar's clock.
    pub text: [u8; 4],
    /// The row labels and the placeholders.
    pub dim: [u8; 4],
    /// Empty segments and the dividers between columns.
    pub track: [u8; 4],
    /// The background under the mouse.
    pub hover: [u8; 4],
}

impl TaskbarTheme {
    /// The colors that read on this taskbar, like Windows' own text on it.
    pub fn palette(self) -> WidgetPalette {
        match self {
            TaskbarTheme::Light => WidgetPalette {
                text: [0x1b, 0x1b, 0x1b, 255],
                dim: [0x5f, 0x64, 0x6c, 255],
                track: [0, 0, 0, 33],
                hover: [0, 0, 0, 14],
            },
            TaskbarTheme::Dark => WidgetPalette {
                text: [255, 255, 255, 255],
                dim: [0xa7, 0xac, 0xb4, 255],
                track: [255, 255, 255, 43],
                hover: [255, 255, 255, 18],
            },
        }
    }
}

/// The opacity of a faded (stale) column.
pub const FADED_OPACITY: f32 = 0.42;

/// Computes the widget from a usage report, as of `now`. `order` is the accounts' order in
/// `config.toml`.
pub(crate) fn view(
    report: &UsageReport,
    order: &[AccountUuid],
    enabled: bool,
    account_labels: bool,
    language: Language,
    tz: &TimeZone,
    now: Timestamp,
) -> TaskbarWidget {
    let mut accounts: Vec<&AccountUsage> = report.accounts.iter().collect();
    // Stable: an account missing from the order keeps its place among the others, last.
    accounts.sort_by_key(|usage| {
        order
            .iter()
            .position(|uuid| uuid == &usage.account.uuid)
            .unwrap_or(usize::MAX)
    });
    let texts = Texts::new(language, tz.clone(), now);
    let columns: Vec<WidgetColumn> = accounts.iter().map(|usage| column(usage, now)).collect();
    let tooltip = accounts
        .iter()
        .map(|usage| texts.account_line(usage))
        .collect::<Vec<_>>()
        .join("\n");
    let redraw_in = accounts
        .iter()
        .filter_map(|usage| usage.reading.clone())
        .flat_map(|reading| {
            let reading = as_of(reading, now);
            [reading.session, reading.weekly]
        })
        .flatten()
        .filter_map(|window| window.resets_at)
        .map(|at| next_change(at, now))
        .fold(REDRAW_AT_MOST_EVERY, Duration::min);
    TaskbarWidget {
        language,
        enabled,
        account_labels,
        row_labels: ["5h".to_owned(), "7d".to_owned()],
        columns,
        tooltip,
        redraw_in,
    }
}

/// How long until the time to reset of a window resetting at `at` reads differently: the next
/// second in its last minute, the next minute before.
fn next_change(at: Timestamp, now: Timestamp) -> Duration {
    let left = at.duration_since(now).as_millis().max(0) as u64;
    let step = if left <= 60_000 { 1_000 } else { 60_000 };
    // The shown number drops just after `left` reaches a multiple of `step`.
    Duration::from_millis(left % step + 1)
}

fn column(usage: &AccountUsage, now: Timestamp) -> WidgetColumn {
    let reading = usage.reading.clone().map(|r| as_of(r, now));
    let state = match &reading {
        Some(_) if usage.stale => ColumnState::Stale,
        Some(_) => ColumnState::Current,
        None if usage.status == UsageStatus::TokenExpired => ColumnState::TokenRenewal,
        None => ColumnState::NoReading,
    };
    let (session, weekly) = match &reading {
        Some(reading) => (reading.session, reading.weekly),
        None => (None, None),
    };
    let color = &usage.account.color;
    WidgetColumn {
        account: usage.account.uuid.clone(),
        label: usage.account.label.clone(),
        light: ink(color, TaskbarTheme::Light),
        dark: ink(color, TaskbarTheme::Dark),
        state,
        session: row(session, SESSION_LENGTH, now),
        weekly: row(weekly, WEEKLY_LENGTH, now),
    }
}

fn row(window: Option<UsageWindow>, length: SignedDuration, now: Timestamp) -> WidgetRow {
    let utilization = window.map_or(0.0, |w| w.utilization);
    let resets_at = window.and_then(|w| w.resets_at);
    let percent = utilization.clamp(0.0, 100.0).round() as u8;
    WidgetRow {
        percent,
        segments: segments(utilization),
        pace: resets_at.map(|at| {
            let left = at.duration_since(now).as_secs_f64();
            (1.0 - left / length.as_secs_f64()).clamp(0.0, 1.0)
        }),
        reset: TimeToReset::until(resets_at, now),
    }
}

/// One segment for each 10% of `utilization` (rounded), at least one above 0.
fn segments(utilization: f64) -> u8 {
    let segments = (utilization.clamp(0.0, 100.0) / 10.0).round() as u8;
    if utilization > 0.0 {
        segments.max(1)
    } else {
        0
    }
}

/// An account's color as drawn on a taskbar of `theme`: a color too dark for the dark taskbar
/// is lightened for its label, one too light for the light taskbar darkened, its bar slightly.
fn ink(color: &Color, theme: TaskbarTheme) -> AccountInk {
    let rgb = color.rgb();
    let lum = luminance(rgb);
    let (bar, text) = match theme {
        TaskbarTheme::Dark => (
            rgb,
            if lum < 0.25 {
                mix(rgb, [255; 3], 0.38)
            } else {
                rgb
            },
        ),
        TaskbarTheme::Light => (
            if lum > 0.6 {
                mix(rgb, [0; 3], 0.12)
            } else {
                rgb
            },
            if lum > 0.45 {
                mix(rgb, [0; 3], 0.42)
            } else {
                rgb
            },
        ),
    };
    AccountInk {
        bar: hex(bar),
        text: hex(text),
    }
}

/// `a` moved toward `b` by `t` (0–1).
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let channel = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round();
    [channel(0) as u8, channel(1) as u8, channel(2) as u8]
}

fn hex([r, g, b]: [u8; 3]) -> Color {
    format!("#{r:02x}{g:02x}{b:02x}")
        .parse()
        .expect("#rrggbb is a valid color")
}

/// Relative luminance, 0 (black) to 1 (white).
fn luminance(rgb: [u8; 3]) -> f32 {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_round_to_tens_with_at_least_one_above_zero() {
        let cases = [
            (0.0, 0),
            (0.2, 1),
            (4.0, 1),
            (14.9, 1),
            (15.0, 2),
            (45.0, 5),
            (94.0, 9),
            (95.0, 10),
            (100.0, 10),
            (120.0, 10),
        ];
        for (utilization, expected) in cases {
            assert_eq!(segments(utilization), expected, "{utilization}%");
        }
    }

    #[test]
    fn colors_are_adjusted_only_where_they_would_not_read() {
        let blue: Color = "#2563eb".parse().unwrap();
        assert_eq!(ink(&blue, TaskbarTheme::Dark).bar, blue);
        assert_eq!(ink(&blue, TaskbarTheme::Light).bar, blue);
        assert_eq!(ink(&blue, TaskbarTheme::Light).text, blue);
        // A dark blue label is lightened on the dark taskbar.
        let navy: Color = "#1e3a8a".parse().unwrap();
        assert_ne!(ink(&navy, TaskbarTheme::Dark).text, navy);
        assert_eq!(ink(&navy, TaskbarTheme::Dark).bar, navy);
        // A light yellow is darkened on the light taskbar, the label more than the bar.
        let yellow: Color = "#facc15".parse().unwrap();
        let light = ink(&yellow, TaskbarTheme::Light);
        assert!(luminance(light.bar.rgb()) < luminance(yellow.rgb()));
        assert!(luminance(light.text.rgb()) < luminance(light.bar.rgb()));
        assert_eq!(ink(&yellow, TaskbarTheme::Dark).bar, yellow);
        // A green just short of that keeps its bar, but its label is darkened.
        let green: Color = "#0ce938".parse().unwrap();
        assert_eq!(ink(&green, TaskbarTheme::Light).bar, green);
        assert_ne!(ink(&green, TaskbarTheme::Light).text, green);
    }
}
