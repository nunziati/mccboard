//! The taskbar widget's tooltip in each language. Every string is matched for every language,
//! so a missing translation doesn't compile. Statuses and clock times are the tray's.

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::tray::text::{Status, Texts as TrayTexts};
use crate::{AccountUsage, Freshness, Language, RenewalSituation, UsageWindow};

/// Texts in one language, with times relative to `now` and shown in `tz`.
pub(super) struct Texts {
    language: Language,
    now: Timestamp,
    tray: TrayTexts,
}

impl Texts {
    pub fn new(language: Language, tz: TimeZone, now: Timestamp) -> Self {
        Self {
            language,
            now,
            tray: TrayTexts::new(language, tz, now),
        }
    }

    fn pick(&self, en: &'static str, it: &'static str) -> &'static str {
        match self.language {
            Language::English => en,
            Language::Italian => it,
        }
    }

    /// An account's tooltip line: "A · Session 7%, resets at 11:10 · Weekly 23%, resets Thu
    /// 21:00 · Fable 4% · read 9 min ago".
    pub fn account_line(&self, usage: &AccountUsage) -> String {
        let label = &usage.account.label;
        let situation = usage.at(self.now);
        let Some(reading) = situation.reading.clone() else {
            let status = match (&situation.renewal, situation.token_expired()) {
                (RenewalSituation::Off { .. }, true) => self.pick(
                    "token expired, renewed the next time Claude Code runs with this account",
                    "token scaduto, si rinnova la prossima volta che usi Claude Code con questo account",
                ),
                (_, true) => self.pick(
                    "token expired, waiting for Claude Code to renew it",
                    "token scaduto, in attesa che Claude Code lo rinnovi",
                ),
                _ => self
                    .tray
                    .status(Status::of(&situation))
                    .unwrap_or_default(),
            };
            return format!("{label} · {status}");
        };
        let mut parts = vec![
            self.window(
                self.pick("Session", "Sessione"),
                reading.session,
                Some(self.pick("idle", "inattiva")),
            ),
            self.window(self.pick("Weekly", "Settimanale"), reading.weekly, None),
        ];
        parts.extend(reading.models.iter().map(|model| {
            format!(
                "{} {}%",
                model.model,
                model.utilization.clamp(0.0, 100.0).round() as u8
            )
        }));
        if situation.freshness == Freshness::Stale {
            if situation.token_expired() {
                parts.extend(self.tray.status(Status::TokenExpired).map(str::to_owned));
            }
            parts.push(self.age(reading.read_at));
        }
        format!("{label} · {}", parts.join(" · "))
    }

    /// "Session 7%, resets at 11:10", "Weekly 23%, resets Thu 21:00"; without a reset time,
    /// `idle` when given ("Session idle"), else the percentage alone.
    fn window(&self, name: &str, window: Option<UsageWindow>, idle: Option<&str>) -> String {
        let percent = window.map_or(0, |w| w.utilization.clamp(0.0, 100.0).round() as u8);
        match window.and_then(|w| w.resets_at) {
            Some(at) => {
                let resets = if self.tray.is_today(at) {
                    self.pick("resets at", "reset alle")
                } else {
                    self.pick("resets", "reset")
                };
                format!("{name} {percent}%, {resets} {}", self.tray.clock(at))
            }
            None => match idle {
                Some(idle) if percent == 0 => format!("{name} {idle}"),
                _ => format!("{name} {percent}%"),
            },
        }
    }

    /// How old a reading is: "read 9 min ago", "letta 2h 5m fa".
    fn age(&self, read_at: Timestamp) -> String {
        let minutes = self
            .now
            .duration_since(read_at)
            .max(SignedDuration::ZERO)
            .as_secs()
            / 60;
        let (days, hours, minutes) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
        let age = match (days, hours) {
            (0, 0) => format!("{minutes} min"),
            (0, h) => format!("{h}h {minutes}m"),
            (d, h) => format!("{d}{} {h}h", self.pick("d", "g")),
        };
        match self.language {
            Language::English => format!("read {age} ago"),
            Language::Italian => format!("letta {age} fa"),
        }
    }
}
