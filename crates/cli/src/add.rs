//! `mccboard add` and `mccboard login`: add a linked instance, with login, and log an
//! instance in on its own.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;
use mccboard_core::{
    AddReport, AddRequest, Core, LoginReport, LoginRun, LoginTerminal, LoginWarning, RepairOutcome,
};

#[derive(Args)]
pub struct AddArgs {
    /// The account's label ("B"). It names the default folder, ~/.claude-<label>, and becomes
    /// the label of an account mccboard doesn't know yet.
    label: String,
    /// The instance's config directory, instead of ~/.claude-<label>.
    #[arg(long, value_name = "PATH")]
    dir: Option<PathBuf>,
    /// Stop once the instance is set up and registered; log in later with `mccboard login`.
    #[arg(long)]
    no_login: bool,
    /// Print machine-readable JSON on stdout, without asking about unknown items first. The
    /// login (unless --no-login) still runs in this terminal; its output goes to stderr.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct LoginArgs {
    /// The registered instance's config directory.
    #[arg(value_name = "PATH")]
    path: PathBuf,
    /// Print machine-readable JSON on stdout. The login runs in this terminal all the same;
    /// its output goes to stderr.
    #[arg(long)]
    json: bool,
}

pub fn add(core: &Core, args: AddArgs) -> Result<()> {
    let request = AddRequest {
        config_dir: args.dir.as_deref().map(crate::user_path).transpose()?,
        login: !args.no_login,
        // The CLI's login runs in its own terminal, on every OS.
        ..AddRequest::new(args.label.clone(), LoginTerminal::CurrentTerminal)
    };
    // Unknown items of the primary instance are asked about first, so this run links them.
    if !args.json && std::io::stdin().is_terminal() {
        let questions = core.sharing_questions()?;
        if !questions.is_empty() {
            crate::doctor::ask(core, &questions)?;
        }
    }
    if !args.json {
        let dir = match &request.config_dir {
            Some(dir) => dir.display().to_string(),
            None => core.add_folder(&request.label)?.display().to_string(),
        };
        println!("Adding {dir} for account {:?}.", request.label);
        if request.login {
            println!(
                "Claude Code's login opens when the instance is ready: log in in your browser \
                 with the account for {:?}.",
                request.label
            );
        }
    }

    let report = core.add(&request)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_add(&report);
    }
    let failed = report.setup.failed();
    if failed > 0 {
        bail!(
            "{failed} setup item{} failed: run `mccboard add` again to retry",
            if failed == 1 { "" } else { "s" }
        );
    }
    if request.login && report.login.account.is_none() {
        bail!(
            "{} is not logged in: run `mccboard login {}`",
            report.config_dir.display(),
            report.config_dir.display()
        );
    }
    Ok(())
}

pub fn login(core: &Core, args: LoginArgs) -> Result<()> {
    let path = crate::user_path(&args.path)?;
    if !args.json {
        println!(
            "Opening Claude Code's login for {}: log in in your browser.",
            path.display()
        );
    }
    let report = core.login(&path, LoginTerminal::CurrentTerminal)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_login(&report);
    }
    if report.account.is_none() {
        bail!("{} is not logged in", report.config_dir.display());
    }
    Ok(())
}

fn print_add(report: &AddReport) {
    let dir = report.config_dir.display();
    if report.created {
        println!("Created {dir}.");
    } else {
        println!("{dir} exists already: completing what's missing.");
    }
    for item in &report.setup.items {
        match &item.outcome {
            RepairOutcome::Done { backup: None } => println!("  done    [{}]", item.id),
            RepairOutcome::Done {
                backup: Some(backup),
            } => println!(
                "  done    [{}] (the copy that was there is in {})",
                item.id,
                backup.display()
            ),
            RepairOutcome::Failed { reason } => println!("  FAILED  [{}] {reason}", item.id),
        }
    }
    if !report.sharing_questions.is_empty() {
        println!("Not linked until you answer (then run `mccboard add` again):");
        for question in &report.sharing_questions {
            println!(
                "  {}  `mccboard share {} link` or `mccboard share {} private`",
                question.item, question.item, question.item
            );
        }
    }
    print_login(&report.login);
}

fn print_login(report: &LoginReport) {
    let dir = report.config_dir.display();
    match report.run {
        LoginRun::Ran { .. } => {}
        LoginRun::AlreadyLoggedIn => println!("{dir} is logged in already."),
        LoginRun::NotRequested => {
            println!("Set up and registered. Log in with `mccboard login {dir}`.")
        }
    }
    match &report.account {
        Some(account) => {
            let email = account.email.as_deref().unwrap_or(account.uuid.as_str());
            println!(
                "{dir} is registered under account {} ({email}){}. Launcher: {}.",
                account.label,
                if report.new_account { ", new" } else { "" },
                account.launcher_name()
            );
        }
        None if report.run != LoginRun::NotRequested => {
            println!("No account is logged in to {dir}: it stays registered, not logged in.")
        }
        None => {}
    }
    for warning in &report.warnings {
        match warning {
            LoginWarning::AccountHasOtherInstances {
                account,
                label,
                instances,
            } => {
                let name = label.clone().unwrap_or_else(|| account.to_string());
                println!(
                    "! Account {name} already has an instance here: {}. Was the right account used in the browser?",
                    instances
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
}
