//! The setup's labels, in English and Italian, added to the TUI's [`Texts`].

use std::path::Path;

use mccboard_core::{
    CLAUDE_CODE_INSTALL_COMMAND, Language, LauncherPlace, LauncherSync, Repair, SharingAnswer,
    UserPathChange,
};

use super::state::StepId;
use crate::tui::texts::{Failure, Texts};

/// A key hint of the setup: the key, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Continue,
    Install,
    Register,
    LogIn,
    Repair,
    Fix,
    Dashboard,
    Select,
    Check,
    Change,
    Add,
    Link,
    Private,
    Skip,
    NextField,
    AddNow,
    Back,
    Close,
    Steps,
    Quit,
    /// `q` in a setup the dashboard opened.
    ToDashboard,
}

impl Texts {
    fn en_it(self, en: String, it: String) -> String {
        match self.language() {
            Language::English => en,
            Language::Italian => it,
        }
    }

    // --- Without a terminal --------------------------------------------------------------

    /// Why `mccboard setup` stops without a terminal, with the commands that do its steps.
    pub fn setup_needs_terminal(self) -> String {
        let commands = match self.language() {
            Language::English => [
                "mccboard claude-code install --yes",
                "mccboard scan, then mccboard register <path>",
                "mccboard add <label>, and mccboard login <path> for an instance not logged in",
                "mccboard doctor, then mccboard share <item> link|private",
                "mccboard repair",
                "mccboard launcher sync",
                "mccboard settings --renew-expired-tokens on|off",
                "mccboard list",
            ],
            Language::Italian => [
                "mccboard claude-code install --yes",
                "mccboard scan, poi mccboard register <percorso>",
                "mccboard add <etichetta>, e mccboard login <percorso> per un'istanza senza login",
                "mccboard doctor, poi mccboard share <elemento> link|private",
                "mccboard repair",
                "mccboard launcher sync",
                "mccboard settings --renew-expired-tokens on|off",
                "mccboard list",
            ],
        };
        let mut text = self
            .pick(
                "`mccboard setup` needs a terminal. Without one, run its steps as commands:",
                "`mccboard setup` ha bisogno di un terminale. Senza, esegui i suoi passi come \
                 comandi:",
            )
            .to_owned();
        for (number, command) in commands.iter().enumerate() {
            text.push_str(&format!("\n  {}. {command}", number + 1));
        }
        text
    }

    // --- Layout --------------------------------------------------------------------------

    pub fn setup_title(self) -> &'static str {
        self.pick("mccboard setup", "configurazione di mccboard")
    }

    pub fn steps(self) -> &'static str {
        self.pick("Steps", "Passi")
    }

    pub fn step_name(self, step: StepId) -> &'static str {
        match step {
            StepId::ClaudeCode => "Claude Code",
            StepId::Instances => self.instances(),
            StepId::Accounts => self.accounts(),
            StepId::Sharing => self.pick("Sharing", "Condivisione"),
            StepId::Drift => self.pick("Drift", "Differenze"),
            StepId::Launchers => self.pick("Launchers", "Comandi di avvio"),
            StepId::Preferences => self.pick("Preferences", "Preferenze"),
            StepId::Summary => self.pick("Summary", "Riepilogo"),
        }
    }

    pub fn hint(self, hint: Hint) -> (&'static str, &'static str) {
        let enter = self.pick("Enter", "Invio");
        match hint {
            Hint::Continue => (enter, self.pick("continue", "continua")),
            Hint::Install => (enter, self.pick("install", "installa")),
            Hint::Register => (enter, self.pick("register", "registra")),
            Hint::LogIn => (enter, self.pick("log in", "accedi")),
            Hint::Repair => (enter, self.pick("repair", "ripara")),
            Hint::Fix => (enter, self.pick("fix them", "sistemali")),
            Hint::Dashboard => (enter, self.pick("open the dashboard", "apri la dashboard")),
            Hint::Select => ("↑↓", self.pick("select", "seleziona")),
            Hint::Check => (self.pick("Space", "Spazio"), self.pick("check", "spunta")),
            Hint::Change => (self.pick("Space", "Spazio"), self.pick("change", "cambia")),
            Hint::Add => ("a", self.pick("add an account", "aggiungi un account")),
            Hint::Link => ("l", self.pick("link", "collega")),
            Hint::Private => ("p", self.pick("keep private", "tieni privato")),
            Hint::Skip => ("s", self.pick("skip", "salta")),
            Hint::NextField => (enter, self.pick("next", "avanti")),
            Hint::AddNow => (enter, self.pick("add and log in", "aggiungi e accedi")),
            Hint::Back => ("Esc", self.pick("back", "indietro")),
            Hint::Close => ("Esc", self.pick("close", "chiudi")),
            Hint::Steps => ("←→", self.pick("step", "passo")),
            Hint::Quit => ("q", self.pick("quit", "esci")),
            Hint::ToDashboard => (
                "q",
                self.pick("back to the dashboard", "torna alla dashboard"),
            ),
        }
    }

    // --- In the terminal handed to Claude Code -------------------------------------------

    pub fn installer_starts(self) -> String {
        self.en_it(
            format!(
                "mccboard setup runs Claude Code's official installer: \
                 {CLAUDE_CODE_INSTALL_COMMAND}"
            ),
            format!(
                "La configurazione di mccboard esegue l'installer ufficiale di Claude Code: \
                 {CLAUDE_CODE_INSTALL_COMMAND}"
            ),
        )
    }

    pub fn login_starts(self, path: &str) -> String {
        self.en_it(
            format!(
                "mccboard setup opens Claude Code's login for {path}. Log in with the account you \
                 want there: Claude Code gives a link to open in a browser, on this machine or \
                 on another."
            ),
            format!(
                "La configurazione di mccboard apre il login di Claude Code per {path}. Accedi \
                 con l'account che vuoi lì: Claude Code dà un link da aprire in un browser, su \
                 questa macchina o su un'altra."
            ),
        )
    }

    pub fn add_starts(self, path: &str, label: &str) -> String {
        self.en_it(
            format!(
                "mccboard setup prepares {path} for account {label}, then opens Claude Code's \
                 login. Log in with the account for {label}: Claude Code gives a link to open in \
                 a browser, on this machine or on another."
            ),
            format!(
                "La configurazione di mccboard prepara {path} per l'account {label}, poi apre il \
                 login di Claude Code. Accedi con l'account per {label}: Claude Code dà un link \
                 da aprire in un browser, su questa macchina o su un'altra."
            ),
        )
    }

    pub fn back_to_setup(self) -> &'static str {
        self.pick(
            "Press Enter to go back to mccboard setup.",
            "Premi Invio per tornare alla configurazione di mccboard.",
        )
    }

    // --- 1. Claude Code ------------------------------------------------------------------

    pub fn claude_found(self, program: &str, version: Option<&str>) -> String {
        match (version, self.language()) {
            (Some(version), Language::English) => {
                format!("Claude Code is installed: {program}, version {version}.")
            }
            (Some(version), Language::Italian) => {
                format!("Claude Code è installato: {program}, versione {version}.")
            }
            (None, Language::English) => {
                format!("Claude Code is installed: {program} (its version can't be read).")
            }
            (None, Language::Italian) => {
                format!("Claude Code è installato: {program} (impossibile leggerne la versione).")
            }
        }
    }

    pub fn claude_not_on_path(self, dir: &str) -> String {
        self.en_it(
            format!(
                "{dir} isn't on the PATH of this terminal yet. mccboard finds Claude Code there \
                 all the same, but `claude` and the launchers need that folder on the PATH: open \
                 a new terminal, or add it to your PATH as the installer said."
            ),
            format!(
                "{dir} non è ancora nel PATH di questo terminale. mccboard trova lo stesso Claude \
                 Code in quella cartella, ma `claude` e i comandi di avvio ne hanno bisogno nel \
                 PATH: apri un nuovo terminale, o aggiungila al PATH come ha detto l'installer."
            ),
        )
    }

    pub fn claude_missing(self) -> &'static str {
        self.pick(
            "Claude Code isn't installed. mccboard can run its official installer here, in this \
             terminal:",
            "Claude Code non è installato. mccboard può eseguire qui, in questo terminale, il suo \
             installer ufficiale:",
        )
    }

    pub fn install_after_asking(self) -> &'static str {
        self.pick(
            "Enter runs it, after asking you.",
            "Invio lo esegue, dopo avertelo chiesto.",
        )
    }

    pub fn claude_missing_windows(self) -> &'static str {
        self.pick(
            "Claude Code isn't installed. Install it in PowerShell with its official command, \
             then open a new terminal and run `mccboard setup` again:",
            "Claude Code non è installato. Installalo in PowerShell con il suo comando ufficiale, \
             poi apri un nuovo terminale ed esegui di nuovo `mccboard setup`:",
        )
    }

    pub fn confirm_install(self) -> &'static str {
        self.pick(
            "Run Claude Code's official installer in this terminal?",
            "Eseguire l'installer ufficiale di Claude Code in questo terminale?",
        )
    }

    pub fn install_keys(self) -> [(&'static str, &'static str); 2] {
        [
            (self.pick("y/Enter", "s/Invio"), self.pick("run", "esegui")),
            ("n/Esc", self.pick("cancel", "annulla")),
        ]
    }

    pub fn install_cancelled(self) -> &'static str {
        self.pick("Nothing was run.", "Non è stato eseguito nulla.")
    }

    pub fn installed(self) -> &'static str {
        self.pick("Claude Code is installed.", "Claude Code è installato.")
    }

    pub fn installer_found_nothing(self, dir: &str) -> String {
        self.en_it(
            format!(
                "The installer ended without an error, but Claude Code is neither on the PATH \
                 nor in {dir}."
            ),
            format!(
                "L'installer è finito senza errori, ma Claude Code non è né nel PATH né in {dir}."
            ),
        )
    }

    pub fn install_failed(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick(
                "Claude Code isn't installed",
                "Claude Code non è installato"
            ),
            self.failure(failure)
        )
    }

    // --- 2. Instances --------------------------------------------------------------------

    pub fn instances_intro(self) -> &'static str {
        self.pick(
            "Instances found in your home: ~/.claude, the primary instance, and each \
             ~/.claude-*. Check the ones to register.",
            "Istanze trovate nella tua home: ~/.claude, l'istanza principale, e ogni \
             ~/.claude-*. Spunta quelle da registrare.",
        )
    }

    pub fn registered_word(self) -> &'static str {
        self.pick("registered", "registrata")
    }

    pub fn no_primary(self) -> &'static str {
        self.pick(
            "~/.claude, the folder of Claude Code itself, isn't there yet: Claude Code makes it on \
             its first login. Enter opens that login here, in this terminal, and registers the \
             primary instance once an account is logged in.",
            "~/.claude, la cartella di Claude Code stesso, non c'è ancora: Claude Code la crea al \
             suo primo login. Invio apre quel login qui, in questo terminale, e registra \
             l'istanza principale appena un account ha effettuato l'accesso.",
        )
    }

    pub fn registered(self, paths: &str, count: usize) -> String {
        match (count, self.language()) {
            (_, Language::English) => format!("Registered {paths}."),
            (1, Language::Italian) => format!("Istanza registrata: {paths}."),
            (_, Language::Italian) => format!("Istanze registrate: {paths}."),
        }
    }

    pub fn register_failed(self, path: &str, failure: &Failure, registered: &str) -> String {
        let mut text = self.en_it(
            format!("{path} isn't registered: {}", self.failure(failure)),
            format!("{path} non è registrata: {}", self.failure(failure)),
        );
        if !registered.is_empty() {
            text.push(' ');
            text.push_str(&self.en_it(
                format!("Registered before it: {registered}."),
                format!("Registrate prima: {registered}."),
            ));
        }
        text
    }

    // --- 3. Accounts ---------------------------------------------------------------------

    pub fn accounts_intro(self) -> &'static str {
        self.pick(
            "Each account logs in to an instance of its own: the primary instance for the first \
             account, a linked instance for each other one.",
            "Ogni account accede a una sua istanza: l'istanza principale per il primo account, \
             un'istanza collegata per ognuno degli altri.",
        )
    }

    pub fn no_registered_instance(self) -> &'static str {
        self.pick(
            "No instance is registered yet: register the primary instance in step 2, or add an \
             account here.",
            "Nessuna istanza registrata: registra l'istanza principale nel passo 2, o aggiungi \
             qui un account.",
        )
    }

    pub fn label_prompt(self) -> &'static str {
        self.pick(
            "Label of the new account (leave it empty to stop adding):",
            "Etichetta del nuovo account (lasciala vuota per smettere di aggiungere):",
        )
    }

    pub fn folder_prompt(self, label: &str) -> String {
        self.en_it(
            format!("Folder of the instance of {label}, which you may change:"),
            format!("Cartella dell'istanza di {label}, che puoi cambiare:"),
        )
    }

    pub fn logged_in_as(self, path: &str, label: &str, others: &str) -> String {
        let mut text = self.en_it(
            format!("{path} is logged in, with account {label}."),
            format!("{path} ha effettuato l'accesso, con l'account {label}."),
        );
        if !others.is_empty() {
            text.push(' ');
            text.push_str(&self.en_it(
                format!(
                    "Account {label} already has an instance here ({others}): was the right \
                     account used in the browser?"
                ),
                format!(
                    "L'account {label} ha già un'istanza qui ({others}): nel browser è stato \
                     usato l'account giusto?"
                ),
            ));
        }
        text
    }

    pub fn still_not_logged_in(self, path: &str) -> String {
        self.en_it(
            format!("{path} is still not logged in: Enter on it opens the login again."),
            format!("{path} non ha ancora effettuato l'accesso: Invio su di essa riapre il login."),
        )
    }

    pub fn login_failed(self, path: &str, failure: &Failure) -> String {
        self.en_it(
            format!("The login of {path} didn't open: {}", self.failure(failure)),
            format!(
                "Il login di {path} non si è aperto: {}",
                self.failure(failure)
            ),
        )
    }

    pub fn added(self, path: &str, login: &str, failed: usize) -> String {
        let mut text = self.en_it(
            format!("Added {path}. {login}"),
            format!("Aggiunta {path}. {login}"),
        );
        if failed > 0 {
            text.push(' ');
            text.push_str(&match (failed, self.language()) {
                (1, Language::English) => "1 setup item failed: add it again to retry.".to_owned(),
                (n, Language::English) => format!("{n} setup items failed: add it again to retry."),
                (1, Language::Italian) => {
                    "1 elemento della preparazione non è riuscito: aggiungila di nuovo per \
                     riprovare."
                        .to_owned()
                }
                (n, Language::Italian) => format!(
                    "{n} elementi della preparazione non sono riusciti: aggiungila di nuovo per \
                     riprovare."
                ),
            });
        }
        text
    }

    pub fn add_failed(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick("Not added", "Non aggiunta"),
            self.failure(failure)
        )
    }

    // --- 4. Sharing ----------------------------------------------------------------------

    pub fn sharing_intro(self) -> &'static str {
        self.pick(
            "The primary instance has items mccboard doesn't know. Each is asked about once, and \
             the answer, saved in config.toml, holds for every linked instance.",
            "L'istanza principale ha elementi che mccboard non conosce. Per ognuno viene chiesto \
             una volta sola, e la risposta, salvata in config.toml, vale per ogni istanza \
             collegata.",
        )
    }

    /// "Link the folder gsd-core …? (1 of 2)".
    pub fn share_question(self, item: &str, is_dir: bool, number: usize, total: usize) -> String {
        match (is_dir, self.language()) {
            (true, Language::English) => format!(
                "Link the folder {item} in the linked instances, or keep it private? ({number} \
                 of {total})"
            ),
            (false, Language::English) => format!(
                "Link the file {item} in the linked instances, or keep it private? ({number} of \
                 {total})"
            ),
            (true, Language::Italian) => format!(
                "Collegare la cartella {item} nelle istanze collegate, o tenerla privata? \
                 ({number} di {total})"
            ),
            (false, Language::Italian) => format!(
                "Collegare il file {item} nelle istanze collegate, o tenerlo privato? ({number} \
                 di {total})"
            ),
        }
    }

    pub fn nothing_to_ask(self) -> &'static str {
        self.pick(
            "The primary instance has no item mccboard doesn't know: nothing to ask.",
            "L'istanza principale non ha elementi che mccboard non conosce: niente da chiedere.",
        )
    }

    pub fn skipped(self, items: &str) -> String {
        self.en_it(
            format!(
                "Skipped for now: {items}. They stay out of the linked instances until you answer."
            ),
            format!(
                "Saltati per ora: {items}. Restano fuori dalle istanze collegate finché non \
                 rispondi."
            ),
        )
    }

    pub fn shared(self, item: &str, answer: SharingAnswer) -> String {
        match (answer, self.language()) {
            (SharingAnswer::Link, Language::English) => {
                format!("{item} will be linked from the primary instance in every linked instance.")
            }
            (SharingAnswer::Link, Language::Italian) => {
                format!("{item} verrà collegato dall'istanza principale in ogni istanza collegata.")
            }
            (SharingAnswer::Private, Language::English) => {
                format!("{item} stays private to each instance.")
            }
            (SharingAnswer::Private, Language::Italian) => {
                format!("{item} resta privato in ogni istanza.")
            }
        }
    }

    // --- 5. Drift ------------------------------------------------------------------------

    pub fn drift_intro(self) -> &'static str {
        self.pick(
            "Each instance against what its sharing says, with what a repair would do.",
            "Ogni istanza confrontata con la sua condivisione, con quello che farebbe una \
             riparazione.",
        )
    }

    /// What a repair would do to `item`, the targets shown by `path`.
    pub fn repair_action(
        self,
        repair: &Repair,
        item: &str,
        path: &dyn Fn(&Path) -> String,
    ) -> String {
        let text = match repair {
            Repair::CreateLink { target } => self.en_it(
                format!("create {item} as a link to {}.", path(target)),
                format!("crea {item} come collegamento a {}.", path(target)),
            ),
            Repair::ReplaceLink { target } => self.en_it(
                format!("replace it with a single link to {}.", path(target)),
                format!(
                    "sostituiscilo con un collegamento diretto a {}.",
                    path(target)
                ),
            ),
            Repair::BackupAndLink { target } => self.en_it(
                format!(
                    "move this copy to a backup in ~/.mccboard/state/, then link {item} to {}.",
                    path(target)
                ),
                format!(
                    "sposta questa copia in un backup in ~/.mccboard/state/, poi collega {item} \
                     a {}.",
                    path(target)
                ),
            ),
            Repair::MigrateSkills { skills } if skills.is_empty() => self
                .pick(
                    "make skills a real folder in this instance, and keep this account's \
                     skills/synced private.",
                    "rendi skills una cartella vera in questa istanza, e tieni privata la \
                     cartella skills/synced di questo account.",
                )
                .to_owned(),
            Repair::MigrateSkills { skills } => self.en_it(
                format!(
                    "make skills a real folder in this instance with one link per personal skill \
                     ({}), and keep this account's skills/synced private.",
                    skills.join(", ")
                ),
                format!(
                    "rendi skills una cartella vera in questa istanza, con un collegamento per \
                     ogni skill personale ({}), e tieni privata la cartella skills/synced di \
                     questo account.",
                    skills.join(", ")
                ),
            ),
            Repair::MigratePlugins => self
                .pick(
                    "give this instance its own plugins folder. Its plugins are then reinstalled \
                     with Claude Code's own commands.",
                    "dai a questa istanza la sua cartella plugins. I suoi plugin vengono poi \
                     reinstallati con i comandi di Claude Code.",
                )
                .to_owned(),
            Repair::Unlink { target } if item == ".credentials.json" => self.en_it(
                format!(
                    "remove the link to {}, leaving the primary instance's login as it is. This \
                     instance is then logged out: log it in again in step 3.",
                    path(target)
                ),
                format!(
                    "rimuovi il collegamento a {}, lasciando com'è il login dell'istanza \
                     principale. Questa istanza resta poi senza login: accedi di nuovo nel passo \
                     3.",
                    path(target)
                ),
            ),
            Repair::Unlink { target } => self.en_it(
                format!(
                    "remove the link to {}, leaving what it leads to as it is. Claude Code then \
                     creates this instance's own {item}.",
                    path(target)
                ),
                format!(
                    "rimuovi il collegamento a {}, lasciando com'è ciò a cui porta. Poi Claude \
                     Code ricrea {item} solo per questa istanza.",
                    path(target)
                ),
            ),
            Repair::SyncLaunchers => self
                .pick("fix the launchers.", "sistema i comandi di avvio.")
                .to_owned(),
            Repair::AlignKey { key, value } => self.en_it(
                format!(
                    "set {key} to {value} in this instance's .claude.json, leaving every other \
                     key as it is."
                ),
                format!(
                    "imposta {key} a {value} nel .claude.json di questa istanza, lasciando com'è \
                     ogni altra chiave."
                ),
            ),
            Repair::AddMarketplace {
                source: Some(source),
                ..
            } => self.en_it(
                format!(
                    "add it with `claude plugin marketplace add {source}`, run in this instance."
                ),
                format!(
                    "aggiungilo con `claude plugin marketplace add {source}`, eseguito in questa \
                     istanza."
                ),
            ),
            Repair::AddMarketplace { source: None, .. } => self
                .pick(
                    "mccboard can't tell how to add it: add it by hand with `claude plugin \
                     marketplace add`, run in this instance.",
                    "mccboard non sa come aggiungerlo: aggiungilo a mano con `claude plugin \
                     marketplace add`, eseguito in questa istanza.",
                )
                .to_owned(),
            Repair::InstallPlugin { plugin } => self.en_it(
                format!(
                    "install it with `claude plugin install {plugin}`, run in this instance \
                     (adding its marketplace first if needed)."
                ),
                format!(
                    "installalo con `claude plugin install {plugin}`, eseguito in questa istanza \
                     (aggiungendo prima il suo marketplace, se manca)."
                ),
            ),
        };
        format!("{}: {text}", self.pick("Repair", "Riparazione"))
    }

    // --- 6. Launchers --------------------------------------------------------------------

    pub fn launchers_place(self, place: &LauncherPlace, path: &dyn Fn(&Path) -> String) -> String {
        match place {
            LauncherPlace::RcFile { path: rc } => self.en_it(
                format!("Launchers are aliases in mccboard's block of {}.", path(rc)),
                format!(
                    "I comandi di avvio sono alias nel blocco di mccboard in {}.",
                    path(rc)
                ),
            ),
            LauncherPlace::BinFolder {
                path: bin,
                on_user_path: true,
            } => self.en_it(
                format!(
                    "Launchers are programs in {}, which is on your user PATH.",
                    path(bin)
                ),
                format!(
                    "I comandi di avvio sono programmi in {}, che è nel PATH dell'utente.",
                    path(bin)
                ),
            ),
            LauncherPlace::BinFolder {
                path: bin,
                on_user_path: false,
            } => self.en_it(
                format!(
                    "Launchers are programs in {}, which isn't on your user PATH yet: Enter puts \
                     it there.",
                    path(bin)
                ),
                format!(
                    "I comandi di avvio sono programmi in {}, che non è ancora nel PATH \
                     dell'utente: Invio ce lo aggiunge.",
                    path(bin)
                ),
            ),
        }
    }

    pub fn no_launchers_needed(self) -> &'static str {
        self.pick(
            "No launcher is needed while no linked instance is registered. The primary instance \
             uses plain `claude`.",
            "Non serve nessun comando di avvio finché non è registrata un'istanza collegata. \
             L'istanza principale usa `claude` e basta.",
        )
    }

    pub fn launcher_ok(self) -> &'static str {
        self.pick("ok", "a posto")
    }

    pub fn launcher_orphan(self, name: &str) -> String {
        format!(
            "{name} ({})",
            self.pick("not needed any more", "non serve più")
        )
    }

    pub fn use_here_rc(self) -> &'static str {
        self.pick(
            "To use them in this terminal, run the line below. New terminals have them already.",
            "Per usarli in questo terminale, esegui la riga qui sotto. I nuovi terminali li hanno \
             già.",
        )
    }

    pub fn use_here_windows(self) -> &'static str {
        self.pick(
            "To use them in this terminal, run the line below in PowerShell, or open a new \
             terminal.",
            "Per usarli in questo terminale, esegui la riga qui sotto in PowerShell, o apri un \
             nuovo terminale.",
        )
    }

    pub fn launchers_synced(self, sync: &LauncherSync) -> String {
        let (created, updated, removed) =
            (sync.created.len(), sync.updated.len(), sync.removed.len());
        let mut text = self.en_it(
            format!("Launchers updated: {created} created, {updated} fixed, {removed} removed."),
            format!(
                "Comandi di avvio aggiornati: {created} creati, {updated} sistemati, {removed} \
                 rimossi."
            ),
        );
        match &sync.user_path {
            Some(UserPathChange::Added) => {
                text.push(' ');
                text.push_str(self.pick(
                    "Their folder is now on your user PATH.",
                    "La loro cartella ora è nel PATH dell'utente.",
                ));
            }
            Some(UserPathChange::NotAdded { reason }) => {
                text.push(' ');
                text.push_str(&self.en_it(
                    format!(
                        "Their folder couldn't be added to your user PATH ({reason}): add it by \
                         hand."
                    ),
                    format!(
                        "Impossibile aggiungere la loro cartella al PATH dell'utente ({reason}): \
                         aggiungila a mano."
                    ),
                ));
            }
            Some(UserPathChange::AlreadyOn) | None => {}
        }
        text
    }

    pub fn launchers_failed(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick(
                "The launchers aren't updated",
                "I comandi di avvio non sono aggiornati"
            ),
            self.failure(failure)
        )
    }

    // --- 7. Preferences ------------------------------------------------------------------

    pub fn renewal_row(self, on: Option<bool>) -> String {
        let value = match on {
            Some(true) => self.pick("on", "sì"),
            Some(false) => self.pick("off", "no"),
            None => "?",
        };
        format!(
            "{}: {value}",
            self.pick(
                "Claude Code may renew an expired token",
                "Claude Code può rinnovare un token scaduto"
            )
        )
    }

    pub fn renewal_hint(self) -> &'static str {
        self.pick(
            "When an account's token has expired, mccboard runs Claude Code hidden in that \
             instance (claude mcp get), and Claude Code renews the token itself. mccboard never \
             touches the credentials.",
            "Quando il token di un account è scaduto, mccboard avvia Claude Code in quell'istanza, \
             nascosto (claude mcp get), e Claude Code rinnova il token da solo. mccboard non \
             tocca mai le credenziali.",
        )
    }

    pub fn renewal_set(self, on: bool) -> &'static str {
        if on {
            self.pick(
                "Claude Code may now renew an expired token.",
                "Ora Claude Code può rinnovare un token scaduto.",
            )
        } else {
            self.pick(
                "Claude Code won't be asked to renew an expired token: it stays expired until \
                 Claude Code runs in that instance.",
                "A Claude Code non verrà chiesto di rinnovare un token scaduto: resta scaduto \
                 finché Claude Code non viene eseguito in quell'istanza.",
            )
        }
    }

    pub fn preference_not_saved(self, failure: &Failure) -> String {
        format!(
            "{}: {}",
            self.pick("The preference isn't saved", "La preferenza non è salvata"),
            self.failure(failure)
        )
    }

    // --- 8. Summary ----------------------------------------------------------------------

    pub fn no_accounts_yet(self) -> &'static str {
        self.pick(
            "No account yet: log in an instance in step 3.",
            "Ancora nessun account: accedi in un'istanza nel passo 3.",
        )
    }

    pub fn still_to_do(self, steps: &str) -> String {
        self.en_it(
            format!("Still to do: {steps}."),
            format!("Ancora da fare: {steps}."),
        )
    }

    pub fn summary_end(self) -> &'static str {
        self.pick(
            "Enter opens the dashboard, with the usage of each account. You can run `mccboard \
             setup` again at any time.",
            "Invio apre la dashboard, con l'utilizzo di ogni account. Puoi eseguire di nuovo \
             `mccboard setup` in qualsiasi momento.",
        )
    }
}
