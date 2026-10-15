use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// The app's language (story 73). The CLI stays English; the app's windows and tray follow
/// this, saved in `config.toml` as `[preferences] language = "it"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "it")]
    Italian,
}

impl Language {
    /// Every language, in menu order.
    pub const ALL: [Language; 2] = [Language::English, Language::Italian];

    /// The code stored in `config.toml` and used by the UI's dictionaries.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Italian => "it",
        }
    }

    /// The language's own name, as shown in a language picker.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Italian => "Italiano",
        }
    }

    /// The language to use when none is chosen, from a system locale (`it-IT`, `it_CH.UTF-8`,
    /// `en-US`): Italian for any Italian locale, English otherwise.
    pub fn from_locale(locale: &str) -> Language {
        let primary = locale
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default();
        if primary.eq_ignore_ascii_case("it") {
            Language::Italian
        } else {
            Language::English
        }
    }
}

impl FromStr for Language {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Language::ALL
            .into_iter()
            .find(|language| language.code() == s)
            .ok_or_else(|| Error::InvalidLanguage {
                language: s.to_owned(),
            })
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
