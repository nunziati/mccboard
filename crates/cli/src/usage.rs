//! `mccboard usage`: each registered account's usage windows.

use anyhow::Result;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use mccboard_core::{
    AccountUsage, Core, Freshness, RenewalFailure, RenewalSituation, UsageBlocker, UsageFailure,
    UsageReport, UsageWarning, UsageWindow,
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
    // Printed at once: the situation as of the report.
    let situation = usage.as_reported();
    match &account.email {
        Some(email) => println!("{} {}  {email}", account.label, account.color),
        None => println!("{} {}  {}", account.label, account.color, account.uuid),
    }

    if let Some(reading) = &situation.reading {
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
            if situation.freshness == Freshness::Stale {
                " (stale)"
            } else {
                ""
            }
        );
    }

    let status = match &situation.blocker {
        None if situation.freshness != Freshness::NoReading => None,
        None if cached => Some("not read yet; run `mccboard usage` to read it".to_owned()),
        None => Some("not read yet".to_owned()),
        Some(UsageBlocker::TokenExpired) => Some(
            match situation.renewal {
                RenewalSituation::Off { .. } => {
                    "token expired: Claude Code renews it the next time it runs with this account \
                 (or turn renewal on: `mccboard settings --renew-expired-tokens on`)"
                }
                _ => "token expired",
            }
            .to_owned(),
        ),
        Some(UsageBlocker::NotLoggedIn) => {
            Some("no registered instance is logged in with this account".to_owned())
        }
        Some(UsageBlocker::CredentialsUnreadable) => Some(
            "the credentials of this account's instances can't be read \
             (on Windows they may be in the Credential Manager)"
                .to_owned(),
        ),
        Some(UsageBlocker::RefreshFailed { failure }) => {
            Some(format!("last refresh failed: {}", failure_text(failure)))
        }
    };
    if let Some(status) = status {
        println!("  {status}");
    }
    if let Some(renewal) = renewal_text(&situation.renewal) {
        println!("  {renewal}");
    }
    if let Some(at) = usage.next_call_at
        && !usage.called
        && !cached
    {
        println!("  next refresh possible after {}", local(at));
    }
}

/// When Claude Code renewed the account's token, or why it couldn't and when it tries again.
fn renewal_text(renewal: &RenewalSituation) -> Option<String> {
    let next = |at: &Option<Timestamp>| match at {
        Some(at) => format!("next try after {}", local(*at)),
        None => "next try at the next refresh".to_owned(),
    };
    match renewal {
        RenewalSituation::Renewed { at, .. } => {
            Some(format!("token renewed by Claude Code at {}", local(*at)))
        }
        RenewalSituation::Failed {
            config_dir,
            at,
            failure,
            retry_at,
        } => Some(format!(
            "renewal in {} failed at {}: {}; {}",
            config_dir.display(),
            local(*at),
            renewal_failure_text(failure),
            next(retry_at)
        )),
        RenewalSituation::Waiting {
            config_dir,
            next_at,
        } => Some(format!(
            "renewal in {}: {}",
            config_dir.display(),
            next(next_at)
        )),
        RenewalSituation::Off { .. } | RenewalSituation::Quiet => None,
    }
}

fn renewal_failure_text(failure: &RenewalFailure) -> String {
    match failure {
        RenewalFailure::ClaudeNotFound => "Claude Code (claude) is not on the PATH".to_owned(),
        RenewalFailure::TimedOut => "Claude Code didn't finish within a minute".to_owned(),
        RenewalFailure::Exited { code: Some(code) } => format!("Claude Code exited with {code}"),
        RenewalFailure::Exited { code: None } => "Claude Code was stopped".to_owned(),
        RenewalFailure::StillExpired => "the token is still expired".to_owned(),
        RenewalFailure::NotStarted => "Claude Code couldn't be started".to_owned(),
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
