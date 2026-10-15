//! `mccboard`: the command-line front end over mccboard's core.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{Candidate, Core, InstanceKind, LoginState, SystemEnvironment, Warning};

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
    }
    Ok(())
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
                login(c),
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

fn login(c: &Candidate) -> String {
    let mut text = match &c.observation.login {
        LoginState::LoggedIn { expires_at } => {
            format!("logged in (token valid until {})", local(*expires_at))
        }
        LoginState::Expired { .. } => "login expired".to_owned(),
        LoginState::NotLoggedIn => "not logged in".to_owned(),
        LoginState::Unknown => "login unknown".to_owned(),
    };
    for warning in &c.observation.warnings {
        text.push_str(match warning {
            Warning::GlobalConfigUnreadable => "; .claude.json unreadable",
            Warning::CredentialsUnreadable => "; .credentials.json unreadable",
        });
    }
    text
}

fn local(at: Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}
