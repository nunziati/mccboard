//! `mccboard`: the command-line front end over mccboard's core.

mod add;
mod claude_code;
mod desktop;
mod doctor;
mod remove;
mod repair;
mod settings;
mod tui;
mod usage;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{
    Account, AccountStatus, AccountUpdate, AccountUuid, Candidate, CliInstall, Color, Core,
    InstanceKind, InstanceStatus, LauncherPlace, LauncherState, LauncherStatus, LauncherSync,
    LoginState, Observation, Registry, SystemEnvironment, UserPathChange, Warning,
};

#[derive(Parser)]
#[command(name = "mccboard", version, about = "Multi Claude Code dashBoard")]
struct Cli {
    /// Home directory to work in (default: your home directory).
    #[arg(long, global = true, value_name = "DIR")]
    home: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Claude Code itself: whether it is installed, and its official installer.
    ClaudeCode {
        #[command(subcommand)]
        command: claude_code::ClaudeCodeCommand,
    },
    /// List instance candidates in the home: ~/.claude and ~/.claude-*. Registers nothing.
    Scan {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List registered accounts and instances.
    List {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Register instances by config directory, at any path. Registering again accepts the
    /// account an instance is now logged in with.
    Register {
        /// Config directories to register.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a linked instance for an account: create its folder (~/.claude-<label> by
    /// default), link and align it, create its launcher, open Claude Code's login in this
    /// terminal, then
    /// register it under the account it logged in with. Running it again completes what's
    /// missing.
    Add(add::AddArgs),
    /// Open Claude Code's login in a registered instance, in this terminal, then record the
    /// account it logged in with. mccboard never sees the credentials.
    Login(add::LoginArgs),
    /// Forget registered instances. Their folders stay as they are.
    Unregister {
        /// Config directories to forget.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Show each account's usage windows. Refreshes from the usage endpoint first, once per
    /// account and only as often as the rate limits allow. An expired token is first renewed
    /// by Claude Code, run hidden in that instance, unless `mccboard settings
    /// --renew-expired-tokens off`.
    Usage {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Show the last known readings without calling the endpoint.
        #[arg(long)]
        cached: bool,
    },
    /// Manage accounts.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Show each instance's drift against the sharing table, explained, with what a repair
    /// would do. Asks once about unknown items of the primary instance when run in a
    /// terminal. Exits with 2 when there is drift.
    Doctor(doctor::DoctorArgs),
    /// Answer whether an unknown item of the primary instance is linked in every linked
    /// instance or kept private. Remembered in config.toml.
    Share(doctor::ShareArgs),
    /// Repair instances' drift, all of it or one item. Refuses while Claude Code runs in an
    /// instance. Real files a link replaces are backed up under ~/.mccboard/state/backups/.
    Repair(repair::RepairArgs),
    /// Remove linked instances: from mccboard only (their launchers go too, their folders
    /// stay), or with --delete-folder --yes their folders too. The primary instance is never
    /// removed.
    Remove(remove::RemoveArgs),
    /// Launchers: the `claude-b` commands that start Claude Code in linked instances.
    Launcher {
        #[command(subcommand)]
        command: LauncherCommand,
    },
    /// Show mccboard's preferences, or change them. Saved in config.toml.
    Settings(settings::SettingsArgs),
    /// Desktop profiles: copies of the Claude desktop app side by side, each bound to one
    /// account and using its local instance.
    Desktop {
        #[command(subcommand)]
        command: desktop::DesktopCommand,
    },
    /// Show each account's usage windows and each instance's state on one screen, refreshed
    /// every minute within the rate limits. Needs a terminal: without one, use `mccboard
    /// usage` and `mccboard list`.
    Dashboard,
    /// Set mccboard up one step at a time: Claude Code, instances, accounts with their login,
    /// sharing, drift, launchers and preferences, then a summary that opens the dashboard.
    /// Each step shows what is done already, and the setup can stop at any step and run again.
    /// Needs a terminal: without one, it lists the commands that do the same steps.
    Setup,
}

#[derive(Subcommand)]
enum LauncherCommand {
    /// Compare the launchers with the registered linked instances. Exits with 2 when
    /// something needs fixing.
    Status {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create missing launchers, fix stale ones and remove the ones nothing needs. Rename a
    /// launcher with `mccboard account set <account> --launcher <name>`. On Windows, also
    /// copy the CLI shipped with the app into the launchers' folder, so `mccboard` is on the
    /// PATH.
    Sync {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AccountCommand {
    /// Change an account's label, color or launcher name.
    Set {
        /// The account, by UUID or by its current label.
        account: String,
        /// New label (1 to 40 characters).
        #[arg(long)]
        label: Option<String>,
        /// New color, as #rrggbb.
        #[arg(long)]
        color: Option<String>,
        /// Launcher name to use instead of the one derived from the label (not `claude` or
        /// `mccboard`).
        #[arg(long, conflicts_with = "default_launcher")]
        launcher: Option<String>,
        /// Drop the launcher name override, so the name follows the label again.
        #[arg(long)]
        default_launcher: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.is::<doctor::DriftFound>() => ExitCode::from(doctor::DRIFT_EXIT_CODE),
        Err(error) => {
            eprintln!("mccboard: error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let home = match cli.home {
        // Absolute, so the primary instance is recognised by its path.
        Some(home) => std::path::absolute(&home)
            .with_context(|| format!("cannot resolve {}", home.display()))?,
        None => std::env::home_dir().context("cannot find your home directory")?,
    };
    let core = Core::new(home, SystemEnvironment);

    match cli.command {
        Command::Add(args) => add::add(&core, args)?,
        Command::Login(args) => add::login(&core, args)?,
        Command::ClaudeCode { command } => claude_code::run(&core, command)?,
        Command::Doctor(args) => doctor::doctor(&core, args)?,
        Command::Share(args) => doctor::share(&core, args)?,
        Command::Repair(args) => repair::repair(&core, args)?,
        Command::Remove(args) => remove::remove(&core, args)?,
        Command::Desktop { command } => desktop::run(&core, command)?,
        Command::Dashboard => tui::dashboard(&core)?,
        Command::Setup => tui::setup(&core)?,
        Command::Scan { json } => {
            let candidates = core.scan()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&candidates)?);
            } else {
                print_candidates(&core, &candidates);
            }
        }
        Command::List { json } => {
            let registry = core.registry()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&registry)?);
            } else {
                print_registry(&core, &registry);
            }
        }
        Command::Register { paths, json } => {
            let mut registrations = Vec::new();
            for path in paths {
                let registration = core.register(&user_path(&path)?)?;
                if !json {
                    let instance = &registration.instance;
                    println!("Registered {}.", instance.config_dir.display());
                    if let Some(account) = &registration.new_account {
                        println!(
                            "New account {}: label {:?}, color {} (change them with `mccboard account set`).",
                            describe(account),
                            account.label,
                            account.color
                        );
                    }
                    if instance.registered_account.is_none() {
                        println!("Not logged in yet: it will join an account once it is.");
                    }
                    if let Some(launcher) = core.launchers()?.instance(&instance.config_dir) {
                        println!("Launcher: {}.", launcher.name);
                    }
                }
                registrations.push(registration);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&registrations)?);
            }
        }
        Command::Unregister { paths } => {
            for path in paths {
                let path = user_path(&path)?;
                core.unregister(&path)?;
                println!("Unregistered {}; its folder is untouched.", path.display());
            }
        }
        Command::Usage { json, cached } => usage::run(&core, json, cached)?,
        Command::Settings(args) => settings::settings(&core, args)?,
        Command::Account {
            command:
                AccountCommand::Set {
                    account,
                    label,
                    color,
                    launcher,
                    default_launcher,
                    json,
                },
        } => {
            let uuid = find_account(&core.registry()?, &account)?;
            let update = AccountUpdate {
                label,
                color: color.map(|c| c.parse::<Color>()).transpose()?,
                launcher: if default_launcher {
                    Some(None)
                } else {
                    launcher.map(Some)
                },
            };
            let saved = core.update_account(&uuid, update)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&saved)?);
            } else {
                let account = &saved.value;
                println!(
                    "Account {}: label {:?}, color {}, launcher {}.",
                    describe(account),
                    account.label,
                    account.color,
                    account.launcher_name()
                );
                desktop::warn_shortcuts(saved.shortcuts_not_synced.as_ref());
            }
        }
        Command::Launcher {
            command: LauncherCommand::Status { json },
        } => {
            let status = core.launchers()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_launchers(&status);
            }
            if status.drift_count() > 0 {
                std::io::Write::flush(&mut std::io::stdout())?;
                std::process::exit(2);
            }
        }
        Command::Launcher {
            command: LauncherCommand::Sync { json },
        } => {
            let sync = core.sync_launchers()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sync)?);
            } else {
                print_sync(&core.launchers()?.place, &sync);
            }
        }
    }
    Ok(())
}

fn print_launchers(status: &LauncherStatus) {
    match &status.place {
        LauncherPlace::RcFile { path } => {
            println!(
                "Launchers: aliases in mccboard's block of {}.",
                path.display()
            );
        }
        LauncherPlace::BinFolder { path, on_user_path } => println!(
            "Launchers: programs in {}, {}.",
            path.display(),
            if *on_user_path {
                "which is on your user PATH"
            } else {
                "which is NOT on your user PATH"
            }
        ),
    }
    if status.instances.is_empty() && status.orphans.is_empty() {
        println!(
            "No linked instance registered, so no launcher is needed. The primary instance uses plain `claude`."
        );
        return;
    }
    let width = status
        .instances
        .iter()
        .map(|i| i.name.chars().count())
        .chain(status.orphans.iter().map(|o| o.name.chars().count()))
        .max()
        .unwrap_or(0);
    for instance in &status.instances {
        let state = match &instance.state {
            LauncherState::Ok => "ok".to_owned(),
            LauncherState::Missing => "missing".to_owned(),
            LauncherState::Stale { found: Some(found) } => {
                format!("stale: starts {}", found.display())
            }
            LauncherState::Stale { found: None } => "stale: not as mccboard wrote it".to_owned(),
        };
        println!(
            "  {:<width$}  {}  {state}",
            instance.name,
            instance.config_dir.display()
        );
    }
    for orphan in &status.orphans {
        println!(
            "  {:<width$}  {}  not needed by any registered instance",
            orphan.name,
            orphan
                .config_dir
                .as_ref()
                .map_or_else(|| "?".to_owned(), |dir| dir.display().to_string())
        );
    }
    let drift = status.drift_count();
    if drift > 0 {
        println!("{drift} to fix: run `mccboard launcher sync`.");
    }
}

fn print_sync(place: &LauncherPlace, sync: &LauncherSync) {
    let show = |verb: &str, launchers: &[mccboard_core::Launcher]| {
        for launcher in launchers {
            match &launcher.config_dir {
                Some(dir) => println!("{verb} {} ({}).", launcher.name, dir.display()),
                None => println!("{verb} {}.", launcher.name),
            }
        }
    };
    show("Created", &sync.created);
    show("Updated", &sync.updated);
    show("Removed", &sync.removed);
    let changed = !(sync.created.is_empty() && sync.updated.is_empty() && sync.removed.is_empty());
    match (&sync.user_path, place) {
        (Some(UserPathChange::Added), LauncherPlace::BinFolder { path, .. }) => println!(
            "Added {} to your user PATH: open a new terminal to use the launchers.",
            path.display()
        ),
        (Some(UserPathChange::NotAdded { reason }), LauncherPlace::BinFolder { path, .. }) => {
            println!(
                "Couldn't add {} to your user PATH ({reason}); add it by hand.",
                path.display()
            )
        }
        _ => {}
    }
    if !changed {
        println!("Launchers already up to date.");
    } else if let LauncherPlace::RcFile { path } = place {
        println!(
            "Open a new terminal, or run `source {}`, to use them.",
            path.display()
        );
    }
    match (&sync.cli, place) {
        (Some(CliInstall::Copied), LauncherPlace::BinFolder { path, .. }) => println!(
            "Copied this version of mccboard's CLI into {}.",
            path.display()
        ),
        (Some(CliInstall::NotCopied { reason }), _) => {
            println!("mccboard's CLI in the bin folder is left as it was: {reason}.")
        }
        _ => {}
    }
}

/// A path as typed: relative ones are taken from the current directory; `~` is left for the
/// core, which expands it to the home it works in.
fn user_path(path: &Path) -> Result<PathBuf> {
    if path.starts_with("~") || path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
    }
}

/// An account by UUID or by label (case-insensitive).
fn find_account(registry: &Registry, wanted: &str) -> Result<AccountUuid> {
    registry
        .accounts
        .iter()
        .find(|a| a.uuid.as_str() == wanted)
        .or_else(|| {
            registry
                .accounts
                .iter()
                .find(|a| a.label.to_lowercase() == wanted.to_lowercase())
        })
        .map(|a| a.uuid.clone())
        .with_context(|| format!("no registered account with UUID or label {wanted:?}"))
}

fn describe(account: &Account) -> String {
    match &account.email {
        Some(email) => format!("{email} ({})", account.uuid),
        None => account.uuid.to_string(),
    }
}

fn print_registry(core: &Core, registry: &Registry) {
    if registry.instances.is_empty() && registry.accounts.is_empty() {
        println!(
            "Nothing registered yet. Run `mccboard scan`, then `mccboard register <path>`. ({})",
            core.paths().config_file().display()
        );
        return;
    }
    let under = |uuid: Option<&AccountUuid>| -> Vec<&InstanceStatus> {
        registry
            .instances
            .iter()
            .filter(|i| i.registered_account.as_ref() == uuid)
            .collect()
    };
    for account in &registry.accounts {
        println!(
            "{} {}  {}  launcher {}",
            account.label,
            account.color,
            describe(account),
            account.launcher_name()
        );
        print_instances(&under(Some(&account.uuid)));
    }
    let known = |uuid: &AccountUuid| registry.accounts.iter().any(|a| &a.uuid == uuid);
    let mut unknown: Vec<&AccountUuid> = registry
        .instances
        .iter()
        .filter_map(|i| i.registered_account.as_ref())
        .filter(|uuid| !known(uuid))
        .collect();
    unknown.sort();
    unknown.dedup();
    for uuid in unknown {
        println!("{uuid} (account not in config.toml; register an instance again to add it)");
        print_instances(&under(Some(uuid)));
    }
    let none = under(None);
    if !none.is_empty() {
        println!("No account");
        print_instances(&none);
    }
}

fn print_instances(instances: &[&InstanceStatus]) {
    if instances.is_empty() {
        println!("  (no registered instance)");
    }
    let width = instances
        .iter()
        .map(|i| i.config_dir.display().to_string().chars().count())
        .max()
        .unwrap_or(0);
    for instance in instances {
        println!(
            "  {:<7}  {:<width$}  {}",
            kind(instance.kind),
            instance.config_dir.display().to_string(),
            login(&instance.observation)
        );
        let observed = instance
            .observation
            .account
            .as_ref()
            .map(|a| a.email.clone().unwrap_or_else(|| a.uuid.to_string()));
        match (&instance.account_status, observed) {
            (AccountStatus::Changed { .. }, Some(observed)) => println!(
                "    ! now logged in as {observed}, not the account it was registered with; \
                 `mccboard register {}` accepts it",
                instance.config_dir.display()
            ),
            (AccountStatus::Unrecorded, Some(observed)) => println!(
                "    ! logged in as {observed} since registration; `mccboard register {}` records it",
                instance.config_dir.display()
            ),
            _ => {}
        }
    }
}

fn print_candidates(core: &Core, candidates: &[Candidate]) {
    if candidates.is_empty() {
        println!("No instances found in {}.", core.home().display());
        return;
    }
    let rows: Vec<[String; 4]> = candidates
        .iter()
        .map(|c| {
            [
                kind(c.kind).to_owned(),
                c.config_dir.display().to_string(),
                account(c),
                format!(
                    "{}{}",
                    login(&c.observation),
                    if c.registered { "; registered" } else { "" }
                ),
            ]
        })
        .collect();
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let (w0, w1, w2) = (width(0), width(1), width(2));
    for [kind, path, account, login] in &rows {
        println!("{kind:<w0$}  {path:<w1$}  {account:<w2$}  {login}");
    }
}

fn kind(kind: InstanceKind) -> &'static str {
    match kind {
        InstanceKind::Primary => "primary",
        InstanceKind::Linked => "linked",
    }
}

fn account(c: &Candidate) -> String {
    match &c.observation.account {
        Some(account) => match &account.email {
            Some(email) => format!("{email} ({})", account.uuid),
            None => account.uuid.to_string(),
        },
        None => "no account".to_owned(),
    }
}

fn login(observation: &Observation) -> String {
    let mut text = match &observation.login {
        LoginState::LoggedIn { expires_at } => {
            format!("logged in (token valid until {})", local(*expires_at))
        }
        LoginState::Expired { .. } => {
            "token expired (Claude Code renews it the next time it runs)".to_owned()
        }
        LoginState::NotLoggedIn => "not logged in".to_owned(),
        LoginState::Unknown => "login unknown".to_owned(),
    };
    for warning in &observation.warnings {
        text.push_str(match warning {
            Warning::GlobalConfigUnreadable => "; .claude.json unreadable",
            Warning::CredentialsUnreadable => "; .credentials.json unreadable",
            Warning::ConfigDirMissing => "; folder missing",
        });
    }
    text
}

fn local(at: Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}
