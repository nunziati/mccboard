//! `mccboard remove`: remove linked instances from mccboard, or with their folders too.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use mccboard_core::{Core, Error, FolderOutcome, Removal, RemoveLevel};
use serde_json::json;

#[derive(Args)]
pub struct RemoveArgs {
    /// Config directories of the linked instances to remove.
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Also delete each instance's folder, and with it the account's login on this machine.
    /// Needs --yes. Refused while Claude Code runs in the instance.
    #[arg(long)]
    delete_folder: bool,
    /// Confirm --delete-folder: the folders and the logins in them will be gone.
    #[arg(long, requires = "delete_folder")]
    yes: bool,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

pub fn remove(core: &Core, args: RemoveArgs) -> Result<()> {
    let dirs: Vec<PathBuf> = args
        .paths
        .iter()
        .map(|p| crate::user_path(p))
        .collect::<Result<_>>()?;

    if args.delete_folder && !args.yes {
        // Nothing changes without the confirmation: say what would go.
        let registry = core.registry()?;
        println!("This would delete, with everything in them:");
        for dir in &dirs {
            let account = registry
                .instances
                .iter()
                .find(|i| i.config_dir == in_home(core, dir))
                .and_then(|i| i.observation.account.as_ref())
                .map(|a| a.email.clone().unwrap_or_else(|| a.uuid.to_string()));
            match account {
                Some(account) => println!(
                    "  {}: the login of {account} on this machine will be gone",
                    dir.display()
                ),
                None => println!("  {}", dir.display()),
            }
        }
        bail!(
            "deleting a folder needs confirmation: run again with --delete-folder --yes. \
             Nothing was changed"
        );
    }

    let level = if args.delete_folder {
        RemoveLevel::WithFolder
    } else {
        RemoveLevel::FromMccboard
    };
    let results: Vec<(PathBuf, Result<Removal, Error>)> = dirs
        .into_iter()
        .map(|dir| {
            let result = core.remove(&dir, level);
            (dir, result)
        })
        .collect();

    if args.json {
        let out: Vec<_> = results
            .iter()
            .map(|(dir, result)| match result {
                Ok(removal) => json!({ "config_dir": dir, "removal": removal }),
                Err(error) => json!({
                    "config_dir": dir,
                    "error": {
                        "code": error.code(),
                        "message": error.to_string(),
                        "processes": error.processes(),
                    },
                }),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for (dir, result) in &results {
            print_result(dir, result);
        }
    }

    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
    if failed > 0 {
        bail!(
            "{failed} instance{} not removed",
            if failed == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

/// `~/x` as the folder in the home the core works in.
fn in_home(core: &Core, path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => core.home().join(rest),
        Err(_) => path.to_path_buf(),
    }
}

fn print_result(dir: &Path, result: &Result<Removal, Error>) {
    match result {
        Ok(removal) => {
            let path = removal.config_dir.display();
            match removal.folder {
                FolderOutcome::Kept => {
                    println!(
                        "Removed {path} from mccboard, with its launcher; its folder is untouched."
                    )
                }
                FolderOutcome::Deleted => {
                    println!("Removed {path} and its launcher, and deleted its folder.")
                }
                FolderOutcome::AlreadyGone => {
                    println!("Removed {path} and its launcher; its folder was already gone.")
                }
            }
        }
        Err(Error::InstanceInUse { processes, .. }) => {
            println!(
                "{}: not removed, Claude Code is running in this instance:",
                dir.display()
            );
            for p in processes {
                match &p.cwd {
                    Some(cwd) => println!("  pid {} ({}) in {}", p.pid, p.name, cwd.display()),
                    None => println!("  pid {} ({})", p.pid, p.name),
                }
            }
            println!("  Close those sessions, then run `mccboard remove` again.");
        }
        Err(error) => println!("{}: not removed: {error}.", dir.display()),
    }
}
