use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// An account's identity: its claude.ai account UUID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountUuid(String);

impl AccountUuid {
    pub fn new(uuid: impl Into<String>) -> Self {
        Self(uuid.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AccountUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The account an instance is logged in with, as read from its global config. Observed,
/// never owned: the instance stays the same instance if this changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservedAccount {
    pub uuid: AccountUuid,
    /// For display only; never used as identity.
    pub email: Option<String>,
}

/// An account mccboard knows, as registered in `config.toml`: its identity and the account
/// label the user gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uuid: AccountUuid,
    /// The short name shown everywhere ("A"). Display only, never identity.
    pub label: String,
    pub color: Color,
    /// The email last observed for this account. Display only.
    pub email: Option<String>,
    /// The user's override of the launcher name; `None` derives it from the label.
    pub launcher: Option<String>,
}

impl Account {
    /// The launcher command for this account's linked instances: the override if set, else
    /// `claude-<label>` with the label lowercased and reduced to ASCII letters, digits and
    /// dashes (`"Personal Max"` → `claude-personal-max`). A label with nothing left falls back
    /// to the UUID's first block.
    pub fn launcher_name(&self) -> String {
        if let Some(name) = &self.launcher {
            return name.clone();
        }
        let slug = slug(&self.label);
        if slug.is_empty() {
            let prefix: String = self
                .uuid
                .as_str()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect();
            format!("claude-{}", prefix.to_ascii_lowercase())
        } else {
            format!("claude-{slug}")
        }
    }
}

impl Serialize for Account {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct View<'a> {
            uuid: &'a AccountUuid,
            label: &'a str,
            color: &'a Color,
            email: &'a Option<String>,
            launcher: &'a Option<String>,
            launcher_name: String,
        }
        View {
            uuid: &self.uuid,
            label: &self.label,
            color: &self.color,
            email: &self.email,
            launcher: &self.launcher,
            launcher_name: self.launcher_name(),
        }
        .serialize(serializer)
    }
}

/// `text` lowercased and reduced to ASCII letters, digits and single dashes, for launcher
/// names (`"Personal Max"` → `personal-max`). Empty when nothing is left.
pub(crate) fn slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// Changes to an account's label, color and launcher name. `None` leaves a field as it is;
/// `launcher: Some(None)` drops the override, so the name follows the label again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountUpdate {
    pub label: Option<String>,
    pub color: Option<Color>,
    pub launcher: Option<Option<String>>,
}

/// An account color, `#rrggbb` in lowercase.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Color(String);

impl Color {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Color {
    type Err = Error;

    /// Accepts `#rrggbb`, in any case.
    fn from_str(s: &str) -> Result<Self, Error> {
        let s = s.trim();
        match s.strip_prefix('#') {
            Some(hex) if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) => {
                Ok(Color(s.to_ascii_lowercase()))
            }
            _ => Err(Error::InvalidColor {
                color: s.to_owned(),
            }),
        }
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Claude's own orange: the color of the default desktop profile's account in every view
/// model, matching the app's icon, which mccboard leaves alone. It can't be edited.
pub const CLAUDE_ORANGE: &str = "#d97757";

pub(crate) fn claude_orange() -> Color {
    Color(CLAUDE_ORANGE.to_owned())
}

/// Colors suggested for new accounts, in order: distinct, and readable on light and dark.
const PALETTE: [&str; 8] = [
    "#2563eb", // blue
    "#ea580c", // orange
    "#16a34a", // green
    "#9333ea", // purple
    "#dc2626", // red
    "#0d9488", // teal
    "#db2777", // pink
    "#ca8a04", // amber
];

const MAX_LABEL_CHARS: usize = 40;
const MAX_LAUNCHER_CHARS: usize = 64;

/// A label as stored: trimmed, non-empty, short, without control characters.
pub(crate) fn validate_label(label: &str) -> Result<String, Error> {
    let trimmed = label.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_LABEL_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(Error::InvalidLabel {
            label: label.to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

/// A launcher name usable as a shell alias and a file name on every OS, and not `claude`
/// itself, which stays the primary instance's.
pub(crate) fn validate_launcher(name: &str) -> Result<String, Error> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= MAX_LAUNCHER_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.eq_ignore_ascii_case("claude");
    if valid {
        Ok(name.to_owned())
    } else {
        Err(Error::InvalidLauncher {
            name: name.to_owned(),
        })
    }
}

pub(crate) fn same_label(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The label suggested for a newly seen account: the suffix of a `.claude-<suffix>` folder
/// (a single letter uppercased), else the first free letter from `A`.
pub(crate) fn suggest_label(config_dir: &Path, primary: bool, taken: &[&str]) -> String {
    let free = |label: &str| !taken.iter().any(|t| same_label(t, label));
    if !primary {
        let suffix = config_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(".claude-"));
        if let Some(suffix) = suffix {
            let label = if suffix.chars().count() == 1 {
                suffix.to_uppercase()
            } else {
                suffix.to_owned()
            };
            if let Ok(label) = validate_label(&label)
                && free(&label)
            {
                return label;
            }
        }
    }
    ('A'..='Z')
        .map(String::from)
        .chain((2..).map(|n| format!("Account {n}")))
        .find(|label| free(label))
        .expect("an endless sequence has a free label")
}

/// The first palette color no account uses yet; when all are taken, the palette repeats.
pub(crate) fn suggest_color(taken: &[&Color]) -> Color {
    let color = PALETTE
        .iter()
        .find(|hex| !taken.iter().any(|t| t.as_str() == **hex))
        .unwrap_or(&PALETTE[taken.len() % PALETTE.len()]);
    Color((*color).to_owned())
}
