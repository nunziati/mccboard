//! `mccboard claude-code`: whether Claude Code is installed, and its official installer.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Result, bail};
use clap::Subcommand;
use mccboard_core::{
    CLAUDE_CODE_INSTALL_COMMAND, CLAUDE_CODE_WINDOWS_INSTALL_COMMAND, ClaudeCode, Core,
};

#[derive(Subcommand)]
pub enum ClaudeCodeCommand {
    /// Show whether Claude Code is installed, where (on the PATH, else ~/.local/bin) and its
    /// version.
    Status {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run Claude Code's official installer in this terminal (curl -fsSL
    /// https://claude.ai/install.sh | bash), after asking. On Windows, name the PowerShell
    /// command to run instead.
    Install {
        /// Run it without asking: needed where there is no terminal to ask in.
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(core: &Core, command: ClaudeCodeCommand) -> Result<()> {
    match command {
        ClaudeCodeCommand::Status { json } => {
            let claude_code = core.claude_code();
            if json {
                println!("{}", serde_json::to_string_pretty(&claude_code)?);
            } else {
                print_status(&claude_code);
            }
            Ok(())
        }
        ClaudeCodeCommand::Install { yes } => install(core, yes),
    }
}

fn install(core: &Core, yes: bool) -> Result<()> {
    // On Windows the core refuses before running anything and names the PowerShell command:
    // there is nothing to ask.
    if !cfg!(windows) && !yes && !confirmed(&core.claude_code())? {
        println!("Nothing was run.");
        return Ok(());
    }
    let claude_code = core.install_claude_code()?;
    if !claude_code.is_found() {
        bail!(
            "the installer ended without an error, but Claude Code isn't on the PATH or in {}",
            core.home().join(".local").join("bin").display()
        );
    }
    print_status(&claude_code);
    Ok(())
}

/// Asks at the terminal whether to run the installer. Without a terminal no one can answer,
/// and nothing runs without `--yes`.
fn confirmed(claude_code: &ClaudeCode) -> Result<bool> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        bail!(
            "running Claude Code's installer needs a confirmation: run this in a terminal, or \
             with --yes. Nothing was run"
        );
    }
    if let ClaudeCode::Found { program, .. } = claude_code {
        println!(
            "Claude Code is installed already ({}): the installer installs it again.",
            program.display()
        );
    }
    println!("This runs Claude Code's official installer in this terminal:");
    println!("  {CLAUDE_CODE_INSTALL_COMMAND}");
    print!("Run it? [y/N]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn print_status(claude_code: &ClaudeCode) {
    match claude_code {
        ClaudeCode::Found {
            program,
            on_path,
            version,
        } => {
            match version {
                Some(version) => println!(
                    "Claude Code is installed: {}, version {version}.",
                    program.display()
                ),
                None => println!(
                    "Claude Code is installed: {} (its version couldn't be read).",
                    program.display()
                ),
            }
            if !on_path && let Some(dir) = program.parent() {
                println!(
                    "{} isn't on this terminal's PATH, where the login and the launchers look \
                     for claude: open a new terminal, or add the folder to your PATH as the \
                     installer said.",
                    dir.display()
                );
            }
        }
        ClaudeCode::Missing if cfg!(windows) => println!(
            "Claude Code isn't installed. Install it in PowerShell with \
             `{CLAUDE_CODE_WINDOWS_INSTALL_COMMAND}`, then open a new terminal."
        ),
        ClaudeCode::Missing => println!(
            "Claude Code isn't installed. `mccboard claude-code install` runs its official \
             installer: {CLAUDE_CODE_INSTALL_COMMAND}"
        ),
    }
}
