//! `mccboard settings`: mccboard's preferences, saved in `config.toml`.

use anyhow::Result;
use clap::Args;
use clap::builder::BoolishValueParser;
use mccboard_core::Core;
use serde_json::json;

#[derive(Args)]
pub struct SettingsArgs {
    /// Let Claude Code renew an expired token when usage is refreshed: `on` or `off`. mccboard
    /// then runs `claude mcp get` with a missing name in the account's instance, hidden; it
    /// never touches the credentials itself.
    #[arg(long, value_name = "on|off", value_parser = BoolishValueParser::new())]
    renew_expired_tokens: Option<bool>,
    /// Show the taskbar widget, every account's usage windows on the Windows taskbar: `on` or
    /// `off`.
    #[arg(long, value_name = "on|off", value_parser = BoolishValueParser::new())]
    taskbar_widget: Option<bool>,
    /// Show each account's label in the taskbar widget: `on` or `off`.
    #[arg(long, value_name = "on|off", value_parser = BoolishValueParser::new())]
    taskbar_widget_labels: Option<bool>,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Changes the preferences given, then prints them all.
pub fn settings(core: &Core, args: SettingsArgs) -> Result<()> {
    if let Some(on) = args.renew_expired_tokens {
        core.set_renew_expired_tokens(on)?;
    }
    if let Some(on) = args.taskbar_widget {
        core.set_taskbar_widget_on(on)?;
    }
    if let Some(on) = args.taskbar_widget_labels {
        core.set_taskbar_widget_labels(on)?;
    }
    // No language means the app follows the system's.
    let language = core.language()?;
    let renew = core.renew_expired_tokens()?;
    let widget = core.taskbar_widget_on()?;
    let labels = core.taskbar_widget_labels()?;
    if args.json {
        let settings = json!({
            "language": language,
            "renew_expired_tokens": renew,
            "taskbar_widget": widget,
            "taskbar_widget_labels": labels,
        });
        println!("{}", serde_json::to_string_pretty(&settings)?);
        return Ok(());
    }
    match language {
        Some(language) => println!("language: {}", language.code()),
        None => println!("language: the system's"),
    }
    let on_off = |on: bool| if on { "on" } else { "off" };
    println!("renew-expired-tokens: {}", on_off(renew));
    println!("taskbar-widget: {}", on_off(widget));
    println!("taskbar-widget-labels: {}", on_off(labels));
    Ok(())
}
