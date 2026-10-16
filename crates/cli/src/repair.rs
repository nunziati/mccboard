//! `mccboard repair`: fix instances' drift, all of it or one item.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;
use mccboard_core::{Core, Error, InstanceKind, RepairOutcome, RepairReport};
use serde_json::json;

#[derive(Args)]
pub struct RepairArgs {
    /// Config directories to repair (default: every registered instance with drift).
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Repair only this drift item, by the id `mccboard doctor` shows (`link:settings.json`,
    /// `private:downloads`, `config:diffTool`, `plugin:review@tools`).
    /// Needs exactly one PATH.
    #[arg(long, value_name = "ID")]
    item: Option<String>,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

pub fn repair(core: &Core, args: RepairArgs) -> Result<()> {
    if args.item.is_some() && args.paths.len() != 1 {
        bail!("--item needs exactly one instance folder");
    }
    let dirs: Vec<PathBuf> = if args.paths.is_empty() {
        core.doctor()?
            .instances
            .into_iter()
            .filter(|i| i.kind == InstanceKind::Linked && i.drift_count > 0)
            .map(|i| i.config_dir)
            .collect()
    } else {
        args.paths
            .iter()
            .map(|p| crate::user_path(p))
            .collect::<Result<_>>()?
    };

    let results: Vec<(PathBuf, Result<RepairReport, Error>)> = dirs
        .into_iter()
        .map(|dir| {
            let result = core.repair(&dir, args.item.as_deref());
            (dir, result)
        })
        .collect();

    if args.json {
        let out: Vec<_> = results
            .iter()
            .map(|(dir, result)| match result {
                Ok(report) => json!({ "config_dir": dir, "report": report }),
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
    } else if results.is_empty() {
        println!("Nothing to repair.");
    } else {
        for (dir, result) in &results {
            print_result(dir, result);
        }
    }

    let unfinished = results
        .iter()
        .filter(|(_, r)| r.as_ref().map_or(true, |report| report.failed() > 0))
        .count();
    if unfinished > 0 {
        bail!(
            "{unfinished} instance{} not fully repaired",
            if unfinished == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

fn print_result(dir: &std::path::Path, result: &Result<RepairReport, Error>) {
    match result {
        Err(Error::InstanceInUse { processes, .. }) => {
            println!(
                "{}: not repaired, Claude Code is running in this instance:",
                dir.display()
            );
            for p in processes {
                match &p.cwd {
                    Some(cwd) => println!("  pid {} ({}) in {}", p.pid, p.name, cwd.display()),
                    None => println!("  pid {} ({})", p.pid, p.name),
                }
            }
            println!("  Close those sessions, then run `mccboard repair` again.");
        }
        Err(error) => println!("{}: not repaired: {error}.", dir.display()),
        Ok(report) if report.items.is_empty() => {
            println!("{}: nothing to repair.", dir.display());
        }
        Ok(report) => {
            println!("{}:", dir.display());
            for item in &report.items {
                match &item.outcome {
                    RepairOutcome::Done { backup: None } => println!("  done    [{}]", item.id),
                    RepairOutcome::Done {
                        backup: Some(backup),
                    } => println!(
                        "  done    [{}] (the copy that was there is in {})",
                        item.id,
                        backup.display()
                    ),
                    RepairOutcome::Failed { reason } => {
                        println!("  FAILED  [{}] {reason}", item.id)
                    }
                }
            }
            match report.drift.drift_count {
                0 => println!("  No drift left."),
                n => println!(
                    "  {n} drift item{} left: see `mccboard doctor`.",
                    if n == 1 { "" } else { "s" }
                ),
            }
        }
    }
}
