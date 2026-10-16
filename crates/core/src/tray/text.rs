//! The tray's texts in each language. Every string is matched for every language, so a
//! missing translation doesn't compile.

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::{Detail, LanguageChoice, TrayAccount, TrayMenu, TrayWindow, WindowName};
use crate::{Language, RenewalFailure, RenewalOutcome};

/// What the tray says about an account besides its numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    /// A current reading: nothing to say.
    Current,
    Stale,
    TokenExpired,
    NeverRead,
    NotLoggedIn,
    CredentialsUnreadable,
    Failed,
}

/// Texts in one language, with times relative to `now` and shown in `tz`.
pub(super) struct Texts {
    language: Language,
    tz: TimeZone,
    now: Timestamp,
}

impl Texts {
    pub fn new(language: Language, tz: TimeZone, now: Timestamp) -> Self {
        Self { language, tz, now }
    }

    fn it(&self) -> bool {
        self.language == Language::Italian
    }

    fn pick(&self, en: &'static str, it: &'static str) -> &'static str {
        match self.language {
            Language::English => en,
            Language::Italian => it,
        }
    }

    pub fn menu(&self) -> TrayMenu {
        TrayMenu {
            open_instances: self.pick("Open Instances", "Apri Istanze").to_owned(),
            refresh: self.pick("Refresh usage", "Aggiorna l'utilizzo").to_owned(),
            settings: self.pick("Settings", "Impostazioni").to_owned(),
            language: self.pick("Language", "Lingua").to_owned(),
            languages: Language::ALL
                .into_iter()
                .map(|language| LanguageChoice {
                    language,
                    name: language.name().to_owned(),
                    selected: language == self.language,
                })
                .collect(),
            quit: self.pick("Quit", "Esci").to_owned(),
        }
    }

    pub fn no_instances(&self) -> String {
        self.pick(
            "No instance registered yet: open Instances to set one up.",
            "Nessuna istanza registrata: apri Istanze per configurarne una.",
        )
        .to_owned()
    }

    /// Why usage can't be shown, by core error code.
    pub fn unavailable(&self, code: &str) -> String {
        match code {
            "config_corrupt" => self.pick(
                "config.toml isn't valid: fix it by hand.",
                "config.toml non è valido: correggilo a mano.",
            ),
            "config_unreadable" => self.pick(
                "can't read config.toml.",
                "impossibile leggere config.toml.",
            ),
            _ => self.pick("can't show usage.", "impossibile mostrare l'utilizzo."),
        }
        .to_owned()
    }

    /// An account's status in lowercase, or `None` for a current reading.
    fn status(&self, status: Status) -> Option<&'static str> {
        Some(match status {
            Status::Current => return None,
            Status::Stale => self.pick("stale", "non aggiornato"),
            Status::TokenExpired => self.pick("token expired", "token scaduto"),
            Status::NeverRead => self.pick(
                "waiting for a first reading",
                "in attesa della prima lettura",
            ),
            Status::NotLoggedIn => self.pick("not logged in", "accesso non effettuato"),
            Status::CredentialsUnreadable => {
                self.pick("credentials unreadable", "credenziali illeggibili")
            }
            Status::Failed => self.pick("can't read usage", "utilizzo non leggibile"),
        })
    }

    /// The menu heading of an account: "A · 35%", "A · 35% (stale)", "B · not logged in".
    pub fn title(&self, label: &str, percent: Option<u8>, status: Status) -> String {
        match (percent, self.status(status)) {
            (Some(percent), None) => format!("{label} · {percent}%"),
            (Some(percent), Some(status)) => format!("{label} · {percent}% ({status})"),
            (None, status) => format!(
                "{label} · {}",
                status.unwrap_or_else(|| self.status(Status::NeverRead).unwrap_or_default())
            ),
        }
    }

    fn window_name(&self, name: &WindowName) -> String {
        match name {
            WindowName::Session => self.pick("Session", "Sessione").to_owned(),
            WindowName::Weekly => self.pick("Weekly", "Settimanale").to_owned(),
            WindowName::Model { model } => model.clone(),
        }
    }

    /// A short window name for the tooltip: "5h", "7d", or the model.
    fn short_name(&self, name: &WindowName) -> String {
        match name {
            WindowName::Session => "5h".to_owned(),
            WindowName::Weekly => self.pick("7d", "7g").to_owned(),
            WindowName::Model { model } => model.clone(),
        }
    }

    /// A menu line: "Session: 35%, resets in 3h (15:00)".
    pub fn window_line(&self, window: &TrayWindow) -> String {
        let name = match &window.name {
            WindowName::Model { model } => {
                format!("{model} ({})", self.pick("weekly", "settimanale"))
            }
            name => self.window_name(name),
        };
        match window.resets_at {
            Some(at) => format!(
                "{name}: {}%, {} {} ({})",
                window.percent,
                self.pick("resets in", "si azzera tra"),
                self.until(at),
                self.clock(at)
            ),
            None => format!("{name}: {}%", window.percent),
        }
    }

    /// "Last reading: 12:00".
    pub fn read_at(&self, at: Timestamp) -> String {
        format!(
            "{}: {}",
            self.pick("Last reading", "Ultima lettura"),
            self.clock(at)
        )
    }

    /// One tooltip line for an account among several: "A: 5h 35% (3h) · 7d 14% (4d 2h)".
    pub fn account_tooltip(&self, account: &TrayAccount, detail: Detail) -> String {
        let status = super::status_of(account);
        let head = match self.status(status) {
            Some(status) if account.percent.is_some() => {
                format!("{} ({status})", account.account.label)
            }
            _ => account.account.label.clone(),
        };
        let Some(peak) = account.percent else {
            let status = self.status(status).unwrap_or_default();
            return match self.renewal_note(account, detail) {
                Some(note) => format!("{head}: {status} · {note}"),
                None => format!("{head}: {status}"),
            };
        };
        if detail == Detail::Peak {
            return format!("{head}: {peak}%");
        }
        let mut parts: Vec<String> = account
            .windows
            .iter()
            .filter_map(|w| self.tooltip_window(w, detail, true))
            .collect();
        parts.extend(self.renewal_note(account, detail));
        format!("{head}: {}", parts.join(" · "))
    }

    /// The tooltip of a single account: one line per window, without label, then its status.
    pub fn single_tooltip(&self, account: &TrayAccount, detail: Detail) -> Vec<String> {
        let status = super::status_of(account);
        let renewal = (detail == Detail::WithRenewal)
            .then(|| self.single_renewal_note(account))
            .flatten();
        let Some(peak) = account.percent else {
            let status = self.status(status).unwrap_or_default();
            return [capitalize(status)].into_iter().chain(renewal).collect();
        };
        let mut lines: Vec<String> = if detail == Detail::Peak {
            vec![format!("{peak}%")]
        } else {
            account
                .windows
                .iter()
                .filter_map(|w| self.tooltip_window(w, detail, false))
                .collect()
        };
        if let (Some(status), Some(read_at)) = (self.status(status), account.read_at) {
            lines.push(format!(
                "{} · {} {}",
                capitalize(status),
                self.pick("read at", "letto alle"),
                self.clock(read_at)
            ));
        }
        lines.extend(renewal);
        lines
    }

    /// The menu line about the token's renewal: when Claude Code renewed it, or why it
    /// couldn't and when it tries again. `None` when there is nothing to say.
    pub fn renewal_line(&self, account: &TrayAccount) -> Option<String> {
        let renewal = account.renewal.as_ref()?;
        let next = match renewal.next_attempt_at {
            Some(at) => format!(
                "{} {}",
                self.pick("next try at", "nuovo tentativo alle"),
                self.clock(at)
            ),
            None => self
                .pick(
                    "next try at the next refresh",
                    "nuovo tentativo al prossimo aggiornamento",
                )
                .to_owned(),
        };
        match (&renewal.last, account.token_expired) {
            (Some(last), false) if last.outcome == RenewalOutcome::Renewed => Some(format!(
                "{} {}",
                self.pick("Token renewed at", "Token rinnovato alle"),
                self.clock(last.at)
            )),
            (_, false) => None,
            (Some(last), true) => match &last.outcome {
                RenewalOutcome::Failed { failure } => Some(format!(
                    "{} ({}), {next}",
                    self.pick("Renewal failed", "Rinnovo non riuscito"),
                    self.failure(failure)
                )),
                RenewalOutcome::Renewed => Some(format!(
                    "{}: {next}",
                    self.pick("Token renewal", "Rinnovo del token")
                )),
            },
            (None, true) => Some(format!(
                "{}: {next}",
                self.pick("Token renewal", "Rinnovo del token")
            )),
        }
    }

    fn failure(&self, failure: &RenewalFailure) -> String {
        match failure {
            RenewalFailure::ClaudeNotFound => self
                .pick("Claude Code not found", "Claude Code non trovato")
                .to_owned(),
            RenewalFailure::TimedOut => self
                .pick(
                    "Claude Code took too long",
                    "Claude Code ci ha messo troppo",
                )
                .to_owned(),
            RenewalFailure::Exited { code: Some(code) } => format!(
                "{} {code}",
                self.pick(
                    "Claude Code exited with code",
                    "Claude Code è uscito con codice"
                )
            ),
            RenewalFailure::Exited { code: None } => self
                .pick("Claude Code was stopped", "Claude Code è stato interrotto")
                .to_owned(),
            RenewalFailure::StillExpired => self
                .pick("token still expired", "token ancora scaduto")
                .to_owned(),
            RenewalFailure::NotStarted => self
                .pick("Claude Code couldn't start", "Claude Code non è partito")
                .to_owned(),
        }
    }

    /// A short note on the token's renewal for an account's tooltip line, at the most detailed
    /// level only: "renewed 14:32", or "renewal failed, retry 15:00" while it is expired.
    fn renewal_note(&self, account: &TrayAccount, detail: Detail) -> Option<String> {
        if detail != Detail::WithRenewal {
            return None;
        }
        let renewal = account.renewal.as_ref()?;
        let last = renewal.last.as_ref()?;
        match (&last.outcome, account.token_expired) {
            (RenewalOutcome::Renewed, false) => Some(format!(
                "{} {}",
                self.pick("renewed", "rinnovato"),
                self.clock(last.at)
            )),
            (RenewalOutcome::Failed { .. }, true) => Some(match renewal.next_attempt_at {
                Some(at) => format!(
                    "{} {}",
                    self.pick("renewal failed, retry", "rinnovo fallito, riprovo"),
                    self.clock(at)
                ),
                None => self.pick("renewal failed", "rinnovo fallito").to_owned(),
            }),
            _ => None,
        }
    }

    /// The renewal note of a single account's tooltip, as a line of its own.
    fn single_renewal_note(&self, account: &TrayAccount) -> Option<String> {
        let note = self.renewal_note(account, Detail::WithRenewal)?;
        let renewed = !account.token_expired;
        Some(if renewed {
            self.renewal_line(account).unwrap_or(note)
        } else {
            capitalize(&note)
        })
    }

    /// A window in the tooltip at the given detail, or `None` when it is left out. Compact
    /// windows read "5h 35% (3h)"; full ones "Session 35% · resets in 3h".
    fn tooltip_window(&self, window: &TrayWindow, detail: Detail, compact: bool) -> Option<String> {
        let model = matches!(window.name, WindowName::Model { .. });
        if model && detail >= Detail::NoModels {
            return None;
        }
        let resets = match detail {
            Detail::WithRenewal | Detail::Full => true,
            Detail::ModelsWithoutResets => !model,
            _ => false,
        };
        let reset = window.resets_at.filter(|_| resets);
        Some(if compact {
            let name = self.short_name(&window.name);
            match reset {
                Some(at) => format!("{name} {}% ({})", window.percent, self.until(at)),
                None => format!("{name} {}%", window.percent),
            }
        } else {
            let name = self.window_name(&window.name);
            match reset {
                Some(at) => format!(
                    "{name} {}% · {} {}",
                    window.percent,
                    self.pick("resets in", "si azzera tra"),
                    self.until(at)
                ),
                None => format!("{name} {}%", window.percent),
            }
        })
    }

    /// How long until `at`: "45m", "2h 15m", "4d 2h" (Italian "4g 2h").
    fn until(&self, at: Timestamp) -> String {
        let left = at.duration_since(self.now).max(SignedDuration::ZERO);
        let minutes = left.as_secs().div_euclid(60).max(1);
        let (days, hours, minutes) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
        let day = if self.it() { "g" } else { "d" };
        match (days, hours, minutes) {
            (0, 0, m) => format!("{m}m"),
            (0, h, 0) => format!("{h}h"),
            (0, h, m) => format!("{h}h {m}m"),
            (d, 0, _) => format!("{d}{day}"),
            (d, h, _) => format!("{d}{day} {h}h"),
        }
    }

    /// A time in the time zone: "15:00" today, "Mon 14:00" on another day.
    fn clock(&self, at: Timestamp) -> String {
        let zoned = at.to_zoned(self.tz.clone());
        let today = self.now.to_zoned(self.tz.clone()).date();
        let time = format!("{:02}:{:02}", zoned.hour(), zoned.minute());
        if zoned.date() == today {
            return time;
        }
        let weekday = zoned.weekday().to_monday_zero_offset() as usize;
        let names = match self.language {
            Language::English => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            Language::Italian => ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
        };
        format!("{} {time}", names[weekday])
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
