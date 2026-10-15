//! Aligned items (ADR 0005): things each instance keeps its own copy of, which mccboard
//! compares with the primary instance and brings in line.
//!
//! - The Global config keys of `.claude.json` ([`GLOBAL_CONFIG_KEYS`]): a key the primary
//!   sets and a linked instance doesn't, or sets differently, is drift (`config:<key>`), with
//!   both values. Repair writes only that key and leaves every other one as it was.
//! - Installed plugins and marketplaces: those the primary has (plugins installed for the
//!   user) and a linked instance lacks are drift (`marketplace:<name>`, `plugin:<id>`).
//!   Repair runs Claude Code's own `claude plugin` commands in the instance through the
//!   environment port, and never copies files from `plugins/`.
//!
//! Detection reads files only; it never runs `claude`, which writes into the config directory
//! it runs in.

mod global_config;
mod plugins;

use std::path::Path;

use crate::doctor::DriftItem;

pub use global_config::GLOBAL_CONFIG_KEYS;
pub(crate) use global_config::align_key;
pub(crate) use plugins::PluginRun;

/// The aligned-item drift of the linked instance in `config_dir`, against the primary
/// instance in `primary` (`~/.claude`): Global config keys in [`GLOBAL_CONFIG_KEYS`] order,
/// then marketplaces, then plugins, each by name.
pub(crate) fn drift(home: &Path, primary: &Path, config_dir: &Path) -> Vec<DriftItem> {
    let mut items = global_config::drift(home, config_dir);
    items.extend(plugins::drift(primary, config_dir));
    items
}
