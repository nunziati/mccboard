//! `mccboard doctor` and `mccboard share`: drift of instances and answers about unknown items.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, ValueEnum};
use mccboard_core::{Core, Doctor, InstanceDrift, InstanceKind, SharingAnswer, SharingQuestion};

/// Exit code of `mccboard doctor` when an instance has drift (errors exit with 1).
pub const DRIFT_EXIT_CODE: u8 = 2;

/// Returned by `doctor` when drift exists: `main` exits with [`DRIFT_EXIT_CODE`], printing
/// nothing more.
#[derive(Debug)]
pub struct DriftFound;

impl std::fmt::Display for DriftFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("drift found")
    }
}

impl std::error::Error for DriftFound {}

#[derive(Args)]
pub struct DoctorArgs {
    /// Config directories to check, registered or not (default: every registered instance).
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Print machine-readable JSON. Asks nothing.
    #[arg(long)]
    json: bool,
    /// Don't ask about unknown items, even in a terminal; just list them.
    #[arg(long)]
    no_ask: bool,
}

#[derive(Args)]
pub struct ShareArgs {
    /// An unknown item of the primary instance, by name (`gsd-core`).
    item: String,
    /// Link it from the primary instance in every linked instance, or keep it private.
    answer: Answer,
}

#[derive(Clone, Copy, ValueEnum)]
enum Answer {
    Link,
    Private,
}

impl From<Answer> for SharingAnswer {
    fn from(answer: Answer) -> Self {
        match answer {
            Answer::Link => SharingAnswer::Link,
            Answer::Private => SharingAnswer::Private,
        }
    }
}

pub fn share(core: &Core, args: ShareArgs) -> Result<()> {
    let answer = SharingAnswer::from(args.answer);
    core.answer_sharing(&args.item, answer)?;
    match answer {
        SharingAnswer::Link => println!(
            "{} will be linked from the primary instance in every linked instance.",
            args.item
        ),
        SharingAnswer::Private => println!("{} stays private to each instance.", args.item),
    }
    Ok(())
}

pub fn doctor(core: &Core, args: DoctorArgs) -> Result<()> {
    let paths = args
        .paths
        .iter()
        .map(|p| crate::user_path(p))
        .collect::<Result<Vec<_>>>()?;
    let check = || -> Result<Doctor> {
        if paths.is_empty() {
            return Ok(core.doctor()?);
        }
        let instances = paths
            .iter()
            .map(|p| core.drift(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Doctor {
            drift_count: instances.iter().map(|i| i.drift_count).sum(),
            questions: core.sharing_questions()?,
            instances,
        })
    };

    let mut report = check()?;
    let interactive = !args.json && !args.no_ask && std::io::stdin().is_terminal();
    if interactive && !report.questions.is_empty() && ask(core, &report.questions)? {
        report = check()?;
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(core, &report, paths.is_empty());
    }
    if report.has_drift() {
        return Err(DriftFound.into());
    }
    Ok(())
}

/// Asks about each unknown item; returns whether any answer was recorded.
pub fn ask(core: &Core, questions: &[SharingQuestion]) -> Result<bool> {
    let stdin = std::io::stdin();
    let mut answered = false;
    println!("The primary instance has items mccboard doesn't know. Each is asked about once.");
    for question in questions {
        let what = if question.is_dir { "folder" } else { "file" };
        loop {
            print!(
                "Share {what} {} with linked instances? [l]ink / keep [p]rivate / [s]kip: ",
                question.item
            );
            std::io::stdout().flush()?;
            let mut line = String::new();
            if stdin.lock().read_line(&mut line)? == 0 {
                println!();
                return Ok(answered);
            }
            let answer = match line.trim().to_lowercase().as_str() {
                "l" | "link" => Some(SharingAnswer::Link),
                "p" | "private" => Some(SharingAnswer::Private),
                "s" | "skip" | "" => None,
                _ => continue,
            };
            if let Some(answer) = answer {
                core.answer_sharing(&question.item, answer)?;
                answered = true;
            }
            break;
        }
    }
    println!();
    Ok(answered)
}

fn print_report(core: &Core, report: &Doctor, registered: bool) {
    if report.instances.is_empty() && registered {
        println!(
            "No registered instance to check. Run `mccboard scan`, then `mccboard register <path>`. ({})",
            core.paths().config_file().display()
        );
    }
    for instance in &report.instances {
        print_instance(instance);
    }
    if !report.questions.is_empty() {
        println!("Unknown items in the primary instance (link or keep private?):");
        for question in &report.questions {
            println!(
                "  {}{}  answer with `mccboard share {} link` or `mccboard share {} private`",
                question.item,
                if question.is_dir { "/" } else { "" },
                question.item,
                question.item
            );
        }
    }
    let instances = report
        .instances
        .iter()
        .filter(|i| i.drift_count > 0)
        .count();
    if report.has_drift() {
        println!(
            "{} drift item{} in {} instance{}.",
            report.drift_count,
            plural(report.drift_count),
            instances,
            plural(instances)
        );
    } else if !report.instances.is_empty() {
        println!("No drift.");
    }
}

fn print_instance(instance: &InstanceDrift) {
    let kind = match instance.kind {
        InstanceKind::Primary => "primary",
        InstanceKind::Linked => "linked",
    };
    let state = match instance.drift_count {
        0 => "no drift".to_owned(),
        n => format!("{n} drift item{}", plural(n)),
    };
    println!("{} ({kind}): {state}", instance.config_dir.display());
    for item in &instance.items {
        let mark = if item.informational { "i" } else { "-" };
        println!("  {mark} [{}] {}", item.id, item.explanation);
        println!("      Repair: {}", item.repair_explanation);
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
