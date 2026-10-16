//! `mccboard desktop`: desktop profiles, copies of the Claude desktop app each bound to one
//! account, listed, added, opened, changed and removed.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use mccboard_core::{
    AccountUuid, Core, DesktopOpening, DesktopProfile, DesktopProfileRemoval, DesktopProfileUpdate,
    DesktopWarning, FolderOutcome, Language, Registry, RemoveLevel, ShortcutSync, ShortcutWarning,
    ShortcutsNotSynced,
};

/// A yes/no setting on the command line: `on` or `off`.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Switch {
    On,
    Off,
}

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
    /// Open a desktop profile (Windows): the Claude desktop app with the profile's data folder,
    /// its Code tab in the account's local instance. Opening an open profile brings it to the
    /// front. The primary instance's account, or --default, opens the default desktop profile.
    Open {
        /// The profile's account, by UUID or by its label.
        #[arg(required_unless_present = "default", conflicts_with = "default")]
        account: Option<String>,
        /// Open the default desktop profile: the app as it starts normally.
        #[arg(long)]
        default: bool,
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
    /// Look once for a desktop profile the app reopened without mccboard (after an update,
    /// say), whose Code tab may lack its instance. The tray app watches all the time and also
    /// tells a profile an update closed; one look can't.
    Notices {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Make the desktop profiles' Start-menu shortcuts (Windows) match the profiles:
    /// `Claude <label>.lnk`, with the profile's colored icon, opening it through mccboard's
    /// app. Adding, changing and removing a profile does this already; pin a shortcut to the
    /// taskbar by hand (right-click, Pin to taskbar).
    Shortcuts {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Change a desktop profile.
    Set {
        /// The profile's account, by UUID or by its label.
        account: String,
        /// Open the profile when mccboard starts.
        #[arg(long, value_name = "on|off")]
        open_at_start: Switch,
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
            let saved = core.add_desktop_profile(&uuid, folder.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&saved)?);
            } else {
                let profile = &saved.value;
                println!(
                    "Added the desktop profile of {}.",
                    name(&registry, profile.account.as_ref())
                );
                print_profile(&registry, profile);
                warn_shortcuts(saved.shortcuts_not_synced.as_ref());
            }
        }
        DesktopCommand::Open {
            account,
            default: _,
            json,
        } => {
            let opening = match account {
                Some(account) => {
                    let registry = core.registry()?;
                    let uuid = crate::find_account(&registry, &account)?;
                    core.open_desktop_profile(&uuid)?
                }
                None => core.open_default_desktop_profile()?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&opening)?);
            } else {
                print_opening(&opening);
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
            let saved = core.remove_desktop_profile(&uuid, level)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&saved)?);
            } else {
                print_removal(&registry, &saved.value);
                warn_shortcuts(saved.shortcuts_not_synced.as_ref());
            }
        }
        DesktopCommand::Notices { json } => notices(core, json)?,
        DesktopCommand::Shortcuts { json } => {
            let sync = core.sync_desktop_shortcuts()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sync)?);
            } else {
                print_shortcuts(&sync);
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
                open_at_start: Some(open_at_start == Switch::On),
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

/// A change was saved but the Start-menu shortcuts couldn't follow it: a warning, not a
/// failure (the exit code stays 0).
pub(crate) fn warn_shortcuts(warning: Option<&ShortcutsNotSynced>) {
    if let Some(warning) = warning {
        eprintln!(
            "warning: {}. The change is saved; run `mccboard desktop shortcuts` to try the \
             shortcuts again.",
            warning.message
        );
    }
}

/// One look at the desktop app's processes: the CLI is English-only.
fn notices(core: &Core, json: bool) -> Result<()> {
    let notices = core.poll_desktop_profiles(Language::English)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&notices)?);
    } else if notices.notices.is_empty() {
        println!("Every desktop profile that runs was opened by mccboard.");
    } else {
        for notice in &notices.notices {
            println!("{}", notice.message);
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
    if profile.running {
        println!("  running");
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

fn print_opening(opening: &DesktopOpening) {
    let account = opening
        .label
        .clone()
        .or_else(|| opening.account.as_ref().map(ToString::to_string));
    match (opening.default, account) {
        (true, Some(account)) => println!(
            "Opened the default desktop profile (account {account}), pid {}.",
            opening.pid
        ),
        (true, None) => println!("Opened the default desktop profile, pid {}.", opening.pid),
        (false, account) => println!(
            "Opened desktop profile {}, pid {}.",
            account.unwrap_or_default(),
            opening.pid
        ),
    }
    if let Some(folder) = &opening.data_folder {
        let created = if opening.created_folder {
            " (created)"
        } else {
            ""
        };
        println!("  data folder    {}{created}", folder.display());
    }
    if let Some(instance) = &opening.instance {
        println!("  Code sessions  {}", instance.display());
    }
    println!("  program        {}", opening.program.display());
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

fn print_shortcuts(sync: &ShortcutSync) {
    for path in &sync.written {
        println!("Wrote {}", path.display());
    }
    for path in &sync.unchanged {
        println!("Up to date: {}", path.display());
    }
    for path in &sync.removed {
        println!("Removed {}", path.display());
    }
    for warning in &sync.warnings {
        match warning {
            ShortcutWarning::NoIcon { account, error } => println!(
                "warning: no colored icon for the profile of {account} ({error}); its shortcut \
                 shows mccboard's icon"
            ),
            ShortcutWarning::NameTaken { path } => println!(
                "warning: {} exists and isn't mccboard's: left as it is, no shortcut written",
                path.display()
            ),
            ShortcutWarning::NotRemoved { path, reason } => {
                println!("warning: cannot remove {}: {reason}", path.display())
            }
            ShortcutWarning::RecordNotSaved { path, reason } => {
                println!("warning: cannot save {}: {reason}", path.display())
            }
        }
    }
    if sync.written.is_empty() && sync.unchanged.is_empty() && sync.removed.is_empty() {
        println!(
            "No desktop profile shortcut (they exist on Windows, for profiles other than the default one)."
        );
    }
}
