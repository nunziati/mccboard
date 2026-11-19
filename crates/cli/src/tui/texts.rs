//! The TUI's labels in each language. Every text is written for every language: a missing
//! translation doesn't compile. Usage texts come from the core's view models (tray and taskbar
//! widget), already in the language asked for: only what the TUI adds is here.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{Language, RepairReport, UsageWarning};

/// Texts in one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Texts {
    language: Language,
}

/// Why an operation failed, as a front end tells it: the core's error code and its message.
/// The message is in English and carries paths only, never a credential (the core's rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl From<&mccboard_core::Error> for Failure {
    fn from(error: &mccboard_core::Error) -> Self {
        Failure {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl Texts {
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    pub fn language(self) -> Language {
        self.language
    }

    /// The text of the language in use, out of its English and Italian versions.
    pub(crate) fn pick(self, en: &'static str, it: &'static str) -> &'static str {
        match self.language {
            Language::English => en,
            Language::Italian => it,
        }
    }

    // --- Any screen ----------------------------------------------------------------------

    /// Why an interactive command stops without a terminal, naming what to run instead.
    pub fn needs_terminal(self, command: &str, instead: &str) -> String {
        match self.language {
            Language::English => {
                format!("`mccboard {command}` needs a terminal. Without one, {instead}.")
            }
            Language::Italian => {
                format!("`mccboard {command}` ha bisogno di un terminale. Senza, {instead}.")
            }
        }
    }

    /// A failure in plain words: the codes a user can act on are translated, the others keep
    /// the core's message.
    pub fn failure(self, failure: &Failure) -> String {
        match failure.code {
            "instance_in_use" => self
                .pick(
                    "Claude Code is running in this instance, so mccboard won't change it. Close \
                     those sessions, then try again. Nothing was changed.",
                    "Claude Code è in esecuzione in questa istanza, quindi mccboard non la \
                     modifica. Chiudi quelle sessioni e riprova. Non è stato cambiato nulla.",
                )
                .to_owned(),
            "developer_mode_off" => self
                .pick(
                    "Linking files on Windows needs Developer Mode, which is off. Turn it on in \
                     Settings › System › For developers, then repair again. Nothing was changed.",
                    "Su Windows i link ai file richiedono la Modalità sviluppatore, che è \
                     disattivata. Attivala in Impostazioni › Sistema › Per sviluppatori, poi \
                     ripara di nuovo. Non è stato cambiato nulla.",
                )
                .to_owned(),
            "config_corrupt" => self
                .pick(
                    "config.toml isn't valid, so mccboard won't change it. Fix it by hand.",
                    "config.toml non è valido, quindi mccboard non lo modifica. Correggilo a mano.",
                )
                .to_owned(),
            "claude_not_installed" => self
                .pick(
                    "Claude Code isn't installed: install it in the first step, then try again.",
                    "Claude Code non è installato: installalo nel primo passo, poi riprova.",
                )
                .to_owned(),
            _ => failure.message.clone(),
        }
    }

    /// A time in `tz`: "15:00".
    pub fn clock(self, at: Timestamp, tz: &TimeZone) -> String {
        at.to_zoned(tz.clone()).strftime("%H:%M").to_string()
    }

    // --- Dashboard: layout ---------------------------------------------------------------

    /// What prints the dashboard's content without a terminal, for [`Texts::needs_terminal`].
    pub fn dashboard_without_terminal(self) -> &'static str {
        self.pick(
            "`mccboard usage` prints each account's usage and `mccboard list` the instances",
            "`mccboard usage` stampa l'utilizzo di ogni account e `mccboard list` le istanze",
        )
    }

    pub fn accounts(self) -> &'static str {
        self.pick("Accounts", "Account")
    }

    pub fn instances(self) -> &'static str {
        self.pick("Instances", "Istanze")
    }

    pub fn refreshing(self) -> &'static str {
        self.pick("Refreshing usage…", "Aggiornamento dell'utilizzo…")
    }

    pub fn session(self) -> &'static str {
        self.pick("Session", "Sessione")
    }

    pub fn weekly(self) -> &'static str {
        self.pick("Weekly", "Settimanale")
    }

    /// The key hints at the bottom of the dashboard, key and action.
    pub fn dashboard_keys(self) -> [(&'static str, &'static str); 6] {
        [
            ("r", self.pick("refresh", "aggiorna")),
            ("p", self.pick("repair", "ripara")),
            ("s", self.pick("setup", "configura")),
            ("l", self.pick("language", "lingua")),
            ("q", self.pick("quit", "esci")),
            ("↑↓", self.pick("select", "seleziona")),
        ]
    }

    pub fn no_instances(self) -> &'static str {
        self.pick(
            "No instance registered yet: run `mccboard scan`, then `mccboard register <path>`.",
            "Nessuna istanza registrata: esegui `mccboard scan`, poi `mccboard register <percorso>`.",
        )
    }

    pub fn no_accounts(self) -> &'static str {
        self.pick(
            "No account yet: log in a registered instance with `mccboard login <path>`.",
            "Ancora nessun account: accedi in un'istanza registrata con `mccboard login <percorso>`.",
        )
    }

    pub fn instances_unreadable(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick("Instances can't be read", "Impossibile leggere le istanze"),
            self.failure(failure)
        )
    }

    // --- Dashboard: instances ------------------------------------------------------------

    /// The column headings of the instances: folder, type, account, login, drift, launcher.
    pub fn instance_columns(self) -> [&'static str; 6] {
        [
            self.pick("Folder", "Cartella"),
            self.pick("Type", "Tipo"),
            "Account",
            "Login",
            self.pick("Drift", "Differenze"),
            self.pick("Launcher", "Comando di avvio"),
        ]
    }

    pub fn primary(self) -> &'static str {
        self.pick("primary", "primaria")
    }

    pub fn linked(self) -> &'static str {
        self.pick("linked", "collegata")
    }

    pub fn no_account(self) -> &'static str {
        self.pick("no account", "nessun account")
    }

    pub fn logged_in(self) -> &'static str {
        self.pick("logged in", "login effettuato")
    }

    pub fn token_expired(self) -> &'static str {
        self.pick("token expired", "token scaduto")
    }

    pub fn not_logged_in(self) -> &'static str {
        self.pick("not logged in", "login non effettuato")
    }

    pub fn login_unknown(self) -> &'static str {
        self.pick("unknown", "sconosciuto")
    }

    pub fn folder_missing(self) -> &'static str {
        self.pick("folder missing", "la cartella non esiste più")
    }

    /// "no drift", "1 item to fix", "3 items to fix".
    pub fn drift(self, count: usize) -> String {
        match (count, self.language) {
            (0, _) => self.pick("no drift", "nessuna differenza").to_owned(),
            (1, Language::English) => "1 item to fix".to_owned(),
            (1, Language::Italian) => "1 elemento da sistemare".to_owned(),
            (n, Language::English) => format!("{n} items to fix"),
            (n, Language::Italian) => format!("{n} elementi da sistemare"),
        }
    }

    pub fn launcher_missing(self, name: &str) -> String {
        format!("{name} ({})", self.pick("missing", "mancante"))
    }

    pub fn launcher_stale(self, name: &str) -> String {
        format!("{name} ({})", self.pick("out of date", "non aggiornato"))
    }

    // --- Dashboard: repair ---------------------------------------------------------------

    pub fn confirm_repair(self, path: &str) -> String {
        match self.language {
            Language::English => format!("Repair {path}?"),
            Language::Italian => format!("Riparare {path}?"),
        }
    }

    /// The line before the drift items a repair would fix.
    pub fn repair_items(self, count: usize) -> String {
        match (count, self.language) {
            (1, Language::English) => "It fixes 1 item:".to_owned(),
            (1, Language::Italian) => "Sistema 1 elemento:".to_owned(),
            (n, Language::English) => format!("It fixes {n} items:"),
            (n, Language::Italian) => format!("Sistema {n} elementi:"),
        }
    }

    pub fn more_items(self, count: usize) -> String {
        match self.language {
            Language::English => format!("… and {count} more"),
            Language::Italian => format!("… e altri {count}"),
        }
    }

    /// The answers to the repair question, key and action.
    pub fn confirm_keys(self) -> [(&'static str, &'static str); 2] {
        [
            (
                self.pick("y/Enter", "s/Invio"),
                self.pick("repair", "ripara"),
            ),
            ("n/Esc", self.pick("cancel", "annulla")),
        ]
    }

    pub fn repairing(self, path: &str) -> String {
        match self.language {
            Language::English => format!("Repairing {path}…"),
            Language::Italian => format!("Riparazione di {path}…"),
        }
    }

    pub fn repair_cancelled(self) -> &'static str {
        self.pick("Repair cancelled.", "Riparazione annullata.")
    }

    /// The user asked to leave while a repair runs: the screen waits for it.
    pub fn leaving_after_repair(self, path: &str) -> String {
        match self.language {
            Language::English => format!("Leaving once the repair of {path} ends…"),
            Language::Italian => format!("Esco appena finisce la riparazione di {path}…"),
        }
    }

    pub fn nothing_to_repair(self, path: &str) -> String {
        match self.language {
            Language::English => format!("Nothing to repair in {path}."),
            Language::Italian => format!("Niente da riparare in {path}."),
        }
    }

    /// What a repair did, in one line.
    pub fn repaired(self, outcome: &RepairOutcome) -> String {
        let path = &outcome.path;
        let mut parts = vec![match (outcome.done, self.language) {
            (1, Language::English) => format!("Repaired 1 item in {path}."),
            (1, Language::Italian) => format!("Riparato 1 elemento in {path}."),
            (n, Language::English) => format!("Repaired {n} items in {path}."),
            (n, Language::Italian) => format!("Riparati {n} elementi in {path}."),
        }];
        if let Some(reason) = &outcome.first_failure {
            parts.push(match self.language {
                Language::English => {
                    format!("{} not repaired: {reason}.", outcome.failed)
                }
                Language::Italian => format!("{} non riparati: {reason}.", outcome.failed),
            });
        }
        if outcome.left > 0 {
            parts.push(format!(
                "{}: {}.",
                self.pick("Drift items left", "Elementi di drift rimasti"),
                outcome.left
            ));
        }
        if outcome.logged_out {
            parts.push(match self.language {
                Language::English => {
                    format!("It is now logged out: log it in again with `mccboard login {path}`.")
                }
                Language::Italian => {
                    format!("Ora è senza login: accedi di nuovo con `mccboard login {path}`.")
                }
            });
        }
        parts.join(" ")
    }

    pub fn repair_failed(self, path: &str, failure: &Failure) -> String {
        match self.language {
            Language::English => format!("{path} not repaired: {}", self.failure(failure)),
            Language::Italian => format!("{path} non riparata: {}", self.failure(failure)),
        }
    }

    // --- Dashboard: refresh and language --------------------------------------------------

    pub fn refreshed(self) -> &'static str {
        self.pick("Usage refreshed.", "Utilizzo aggiornato.")
    }

    /// No account could be read yet: the rate limits allow the next call at `time`.
    pub fn next_call_at(self, time: &str) -> String {
        match self.language {
            Language::English => format!(
                "The usage endpoint allows the next call at {time}: the last readings stay."
            ),
            Language::Italian => format!(
                "L'endpoint dell'utilizzo consente la prossima chiamata alle {time}: restano le \
                 ultime letture."
            ),
        }
    }

    pub fn refresh_failed(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick(
                "Usage can't be refreshed",
                "Impossibile aggiornare l'utilizzo"
            ),
            self.failure(failure)
        )
    }

    pub fn usage_warning(self, warning: UsageWarning) -> &'static str {
        match warning {
            UsageWarning::StateDiscarded => self.pick(
                "The saved usage readings were unreadable and have been discarded.",
                "Le letture salvate dell'utilizzo non erano leggibili e sono state scartate.",
            ),
            UsageWarning::StateUnwritable => self.pick(
                "Usage readings can't be saved: they won't survive a restart.",
                "Le letture dell'utilizzo non si possono salvare: non sopravvivranno a un riavvio.",
            ),
        }
    }

    /// The language now in use, in its own words: "Language: English", "Lingua: Italiano".
    pub fn language_set(self) -> String {
        format!(
            "{}: {}",
            self.pick("Language", "Lingua"),
            self.language.name()
        )
    }

    pub fn language_not_saved(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick(
                "The language is used but not saved",
                "La lingua è in uso ma non salvata"
            ),
            self.failure(failure)
        )
    }
}

/// What a repair did, for [`Texts::repaired`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairOutcome {
    /// The instance's folder as shown.
    pub path: String,
    pub done: usize,
    pub failed: usize,
    /// Why the first failed item wasn't repaired.
    pub first_failure: Option<String>,
    /// Drift items left after the repair.
    pub left: usize,
    /// The repair left the instance logged out.
    pub logged_out: bool,
}

impl RepairOutcome {
    /// What `report` did, in the instance shown as `path`.
    pub fn of(path: String, report: &RepairReport) -> RepairOutcome {
        let failures: Vec<&str> = report
            .items
            .iter()
            .filter_map(|item| match &item.outcome {
                mccboard_core::RepairOutcome::Failed { reason } => Some(reason.as_str()),
                mccboard_core::RepairOutcome::Done { .. } => None,
            })
            .collect();
        RepairOutcome {
            path,
            done: report.items.len() - failures.len(),
            failed: failures.len(),
            first_failure: failures.first().map(|reason| (*reason).to_owned()),
            left: report.drift.drift_count,
            logged_out: report.items.iter().any(|item| item.logged_out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_failures_keep_the_core_message() {
        let failure = Failure {
            code: "not_registered",
            message: "/home/u/.claude-x is not registered".to_owned(),
        };
        for language in Language::ALL {
            assert_eq!(
                Texts::new(language).failure(&failure),
                "/home/u/.claude-x is not registered"
            );
        }
    }

    #[test]
    fn drift_counts_read_naturally() {
        let en = Texts::new(Language::English);
        let it = Texts::new(Language::Italian);
        assert_eq!(en.drift(0), "no drift");
        assert_eq!(en.drift(1), "1 item to fix");
        assert_eq!(en.drift(3), "3 items to fix");
        assert_eq!(it.drift(0), "nessuna differenza");
        assert_eq!(it.drift(1), "1 elemento da sistemare");
        assert_eq!(it.drift(3), "3 elementi da sistemare");
    }
}
