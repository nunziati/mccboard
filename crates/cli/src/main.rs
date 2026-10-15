//! `mccboard`: the command-line front end over mccboard's core.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{
    Account, AccountStatus, AccountUpdate, AccountUuid, Candidate, Color, Core, InstanceKind,
    InstanceStatus, LoginState, Observation, Registry, SystemEnvironment, Warning,
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
    /// Forget registered instances. Their folders stay as they are.
    Unregister {
        /// Config directories to forget.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Manage accounts.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
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
        /// Launcher name to use instead of the one derived from the label.
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
        Err(error) => {
            eprintln!("mccboard: error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let home = match cli.home {
        Some(home) => home,
        None => std::env::home_dir().context("cannot find your home directory")?,
    };
    let core = Core::new(home, SystemEnvironment);

    match cli.command {
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
            let account = core.update_account(&uuid, update)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&account)?);
            } else {
                println!(
                    "Account {}: label {:?}, color {}, launcher {}.",
                    describe(&account),
                    account.label,
                    account.color,
                    account.launcher_name()
                );
            }
        }
    }
    Ok(())
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
        LoginState::Expired { .. } => "login expired".to_owned(),
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
