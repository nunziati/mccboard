//! `mccboard usage`: each registered account's usage windows.

use anyhow::Result;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{
    AccountUsage, Core, UsageFailure, UsageReport, UsageStatus, UsageWarning, UsageWindow,
};

/// Refreshes usage within the rate limits (or, with `cached`, only reads the last readings)
/// and prints it.
pub fn run(core: &Core, json: bool, cached: bool) -> Result<()> {
    let report = if cached {
        core.usage()?
    } else {
        core.refresh_usage()?
    };
    for warning in &report.warnings {
        eprintln!("mccboard: warning: {}", warning_text(*warning));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print(&report, cached);
    }
    Ok(())
}

fn warning_text(warning: UsageWarning) -> &'static str {
    match warning {
        UsageWarning::StateDiscarded => {
            "the saved usage readings were unreadable and have been discarded"
        }
        UsageWarning::StateUnwritable => {
            "cannot save usage readings; they won't survive a restart and rate limits may reset"
        }
    }
}

fn print(report: &UsageReport, cached: bool) {
    if report.accounts.is_empty() {
        println!(
            "No account registered yet. Run `mccboard scan`, then `mccboard register <path>`."
        );
        return;
    }
    for (index, usage) in report.accounts.iter().enumerate() {
        if index > 0 {
            println!();
        }
        print_account(usage, cached);
    }
}

fn print_account(usage: &AccountUsage, cached: bool) {
    let account = &usage.account;
    match &account.email {
        Some(email) => println!("{} {}  {email}", account.label, account.color),
        None => println!("{} {}  {}", account.label, account.color, account.uuid),
    }

    if let Some(reading) = &usage.reading {
        let mut rows: Vec<(String, f64, Option<Timestamp>)> = Vec::new();
        let named = [("session", &reading.session), ("weekly", &reading.weekly)];
        for (name, window) in named {
            if let Some(UsageWindow {
                utilization,
                resets_at,
            }) = window
            {
                rows.push((name.to_owned(), *utilization, *resets_at));
            }
        }
        for model in &reading.models {
            rows.push((
                format!("{} (weekly)", model.model),
                model.utilization,
                model.resets_at,
            ));
        }
        let width = rows
            .iter()
            .map(|(name, ..)| name.chars().count())
            .max()
            .unwrap_or(0);
        for (name, utilization, resets_at) in rows {
            let resets = match resets_at {
                Some(at) => format!("resets {}", local(at)),
                None => String::new(),
            };
            println!("  {name:<width$}  {utilization:>3.0}%  {resets}");
        }
        println!(
            "  read {}{}",
            local(reading.read_at),
            if usage.stale { " (stale)" } else { "" }
        );
    }

    let status = match &usage.status {
        UsageStatus::Ok => None,
        UsageStatus::NeverRead if cached => {
            Some("not read yet; run `mccboard usage` to read it".to_owned())
        }
        UsageStatus::NeverRead => Some("not read yet".to_owned()),
        UsageStatus::LoginExpired => Some(
            "login expired: Claude Code renews it the next time it runs with this account"
                .to_owned(),
        ),
        UsageStatus::NotLoggedIn => {
            Some("no registered instance is logged in with this account".to_owned())
        }
        UsageStatus::CredentialsUnreadable => Some(
            "the credentials of this account's instances can't be read \
             (on Windows they may be in the Credential Manager)"
                .to_owned(),
        ),
        UsageStatus::Failed { failure } => {
            Some(format!("last refresh failed: {}", failure_text(failure)))
        }
    };
    if let Some(status) = status {
        println!("  {status}");
    }
    if let Some(at) = usage.next_call_at
        && !usage.called
        && !cached
    {
        println!("  next refresh possible after {}", local(at));
    }
}

fn failure_text(failure: &UsageFailure) -> String {
    match failure {
        UsageFailure::RateLimited => "rate limited by the endpoint (429)".to_owned(),
        UsageFailure::Unauthorized => {
            "token refused (401); Claude Code renews it the next time it runs".to_owned()
        }
        UsageFailure::Forbidden => "not allowed (403)".to_owned(),
        UsageFailure::HttpStatus { status } => format!("HTTP status {status}"),
        UsageFailure::Unreachable { reason } => format!("endpoint unreachable ({reason})"),
        UsageFailure::Malformed => "unexpected reply from the endpoint".to_owned(),
    }
}

fn local(at: Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}
