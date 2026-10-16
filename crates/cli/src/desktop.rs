//! `mccboard desktop`: desktop profiles, copies of the Claude desktop app each bound to one
//! account. Nothing is launched from here yet.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use mccboard_core::{
    AccountUuid, Core, DesktopProfile, DesktopProfileRemoval, DesktopProfileUpdate, DesktopWarning,
    FolderOutcome, Registry, RemoveLevel,
};

#[derive(Subcommand)]
pub enum DesktopCommand {
    /// List the desktop profiles, the default desktop profile first, with the account each is
    /// signed in with and what looks wrong.
    List {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a desktop profile for an account that has a local instance: its Code tab will use
    /// that instance. Only config.toml changes; the data folder is created when the profile
    /// first opens.
    Add {
        /// The account, by UUID or by its label.
        account: String,
        /// The profile's data folder (default: %LOCALAPPDATA%\Claude-<label>).
        #[arg(long, value_name = "PATH")]
        folder: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove a desktop profile from mccboard (its data folder stays), or with --with-folder
    /// --yes its data folder too. The default desktop profile is never removed.
    Remove {
        /// The profile's account, by UUID or by its label.
        account: String,
        /// Also delete the data folder, and with it the profile's sign-in on this machine.
        /// Needs --yes. Refused while the profile runs.
        #[arg(long)]
        with_folder: bool,
        /// Confirm --with-folder: the data folder and the sign-in in it will be gone.
        #[arg(long, requires = "with_folder")]
        yes: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Change a desktop profile.
    Set {
        /// The profile's account, by UUID or by its label.
        account: String,
        /// Open the profile when mccboard starts.
        #[arg(long, value_name = "on|off", value_parser = ["on", "off"], required = true)]
        open_at_start: Option<String>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

pub fn run(core: &Core, command: DesktopCommand) -> Result<()> {
    match command {
        DesktopCommand::List { json } => {
            let profiles = core.desktop_profiles()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profiles)?);
            } else {
                let registry = core.registry()?;
                for profile in &profiles {
                    print_profile(&registry, profile);
                }
                if profiles.len() == 1 {
                    println!(
                        "No other desktop profile. Add one with `mccboard desktop add <account>`."
                    );
                }
            }
        }
        DesktopCommand::Add {
            account,
            folder,
            json,
        } => {
            let registry = core.registry()?;
            let uuid = crate::find_account(&registry, &account)?;
            let folder = folder.map(|f| crate::user_path(&f)).transpose()?;
            let profile = core.add_desktop_profile(&uuid, folder.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            } else {
                println!(
                    "Added the desktop profile of {}.",
                    name(&registry, profile.account.as_ref())
                );
                print_profile(&registry, &profile);
            }
        }
        DesktopCommand::Remove {
            account,
            with_folder,
            yes,
            json,
        } => {
            let registry = core.registry()?;
            let uuid = crate::find_account(&registry, &account)?;
            if with_folder && !yes {
                // Nothing changes without the confirmation: say what would go.
                let profiles = core.desktop_profiles()?;
                if let Some(profile) = profiles
                    .iter()
                    .find(|p| !p.default && p.account.as_ref() == Some(&uuid))
                {
                    println!(
                        "This would delete {}, with everything in it: the sign-in of desktop \
                         profile {} on this machine will be gone.",
                        profile.data_folder.display(),
                        name(&registry, Some(&uuid))
                    );
                }
                bail!(
                    "deleting a data folder needs confirmation: run again with --with-folder \
                     --yes. Nothing was changed"
                );
            }
            let level = if with_folder {
                RemoveLevel::WithFolder
            } else {
                RemoveLevel::FromMccboard
            };
            let removal = core.remove_desktop_profile(&uuid, level)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&removal)?);
            } else {
                print_removal(&registry, &removal);
            }
        }
        DesktopCommand::Set {
            account,
            open_at_start,
            json,
        } => {
            let registry = core.registry()?;
            let uuid = crate::find_account(&registry, &account)?;
            let update = DesktopProfileUpdate {
                open_at_start: open_at_start.map(|value| value == "on"),
            };
            let profile = core.update_desktop_profile(&uuid, update)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            } else {
                print_profile(&registry, &profile);
            }
        }
    }
    Ok(())
}

/// An account by label when registered, else by UUID.
fn name(registry: &Registry, account: Option<&AccountUuid>) -> String {
    match account {
        Some(uuid) => registry
            .accounts
            .iter()
            .find(|a| &a.uuid == uuid)
            .map_or_else(|| uuid.to_string(), |a| a.label.clone()),
        None => "no account".to_owned(),
    }
}

fn print_profile(registry: &Registry, profile: &DesktopProfile) {
    let account = name(registry, profile.account.as_ref());
    if profile.default {
        match &profile.account {
            Some(_) => println!(
                "Default desktop profile: account {account} ({}, Claude's orange)",
                profile.color
            ),
            None => {
                println!("Default desktop profile: no account yet (register the primary instance)")
            }
        }
    } else {
        println!(
            "Desktop profile {account} ({}){}",
            profile.color,
            if profile.open_at_start {
                ", opens when mccboard starts"
            } else {
                ""
            }
        );
    }
    println!("  data folder    {}", profile.data_folder.display());
    match &profile.instance {
        Some(dir) => println!("  Code sessions  {}", dir.display()),
        None => println!("  Code sessions  no local instance"),
    }
    match &profile.observed.account {
        Some(observed) => println!("  signed in as   {}", name(registry, Some(observed))),
        None => println!("  signed in as   not signed in yet"),
    }
    if !profile.observed.others.is_empty() {
        let others: Vec<String> = profile
            .observed
            .others
            .iter()
            .map(|uuid| name(registry, Some(uuid)))
            .collect();
        println!("  signed in before as {}", others.join(", "));
    }
    for warning in &profile.warnings {
        let observed = |uuid: &AccountUuid, label: &Option<String>| {
            label.clone().unwrap_or_else(|| uuid.to_string())
        };
        match warning {
            DesktopWarning::ObservedAccountDiffers {
                observed: uuid,
                observed_label,
            } => println!(
                "  ! signed in as {}, not {account}: sign out of it in the app and sign in \
                 with {account}",
                observed(uuid, observed_label)
            ),
            DesktopWarning::DefaultProfileNotPrimaryAccount {
                observed: uuid,
                observed_label,
            } => println!(
                "  ! signed in as {}, not the primary instance's account {account}: its Code \
                 sessions would write that account into ~/.claude.json",
                observed(uuid, observed_label)
            ),
            DesktopWarning::NoLocalInstance => println!(
                "  ! account {account} has no local instance any more: add one with \
                 `mccboard add {account}`"
            ),
        }
    }
}

fn print_removal(registry: &Registry, removal: &DesktopProfileRemoval) {
    let account = name(registry, Some(&removal.account));
    let folder = removal.data_folder.display();
    match removal.folder {
        FolderOutcome::Kept => println!(
            "Removed the desktop profile of {account} from mccboard; its data folder {folder} is \
             untouched."
        ),
        FolderOutcome::Deleted => println!(
            "Removed the desktop profile of {account} and deleted its data folder {folder}."
        ),
        FolderOutcome::AlreadyGone => println!(
            "Removed the desktop profile of {account}; its data folder {folder} was already gone."
        ),
    }
}
