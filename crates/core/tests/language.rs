//! The app's language: Italian or English, chosen in settings and kept in `config.toml`
//! (story 73; ticket #8).

mod common;

use common::{TempHome, UUID_B};
use mccboard_core::{Error, Language};

#[test]
fn no_language_is_chosen_until_the_user_picks_one() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    assert_eq!(core.language().unwrap(), None);
    assert!(home.config_text().is_none());
}

#[test]
fn the_chosen_language_persists_in_config_toml() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    core.set_language(Language::Italian).unwrap();

    let config: toml::Table = home.config_text().unwrap().parse().unwrap();
    assert_eq!(config["preferences"]["language"].as_str(), Some("it"));
    let (restarted, _env) = home.core();
    assert_eq!(restarted.language().unwrap(), Some(Language::Italian));

    core.set_language(Language::English).unwrap();
    assert_eq!(restarted.language().unwrap(), Some(Language::English));
}

#[test]
fn choosing_a_language_keeps_everything_else_in_the_file() {
    let home = TempHome::new();
    let original = format!(
        r##"# My setup
[accounts.{UUID_B}]
label = "Bee" # the work one
color = "#ff8800"
future_key = 1

[preferences]
language = "en" # picked on day one
theme = "dark"
"##
    );
    home.write_config(&original);
    let (core, _env) = home.core();

    core.set_language(Language::Italian).unwrap();

    assert_eq!(
        home.config_text().unwrap(),
        original.replace(r#"language = "en""#, r#"language = "it""#)
    );
}

#[test]
fn a_hand_written_language_is_read() {
    let home = TempHome::new();
    home.write_config("[preferences]\nlanguage = \"it\"\n");
    let (core, _env) = home.core();

    assert_eq!(core.language().unwrap(), Some(Language::Italian));
}

#[test]
fn an_unknown_language_is_reported_and_the_file_left_alone() {
    let home = TempHome::new();
    home.write_config("[preferences]\nlanguage = \"fr\"\n");
    let (core, _env) = home.core();

    let error = core.language().unwrap_err();
    assert!(matches!(error, Error::ConfigCorrupt { .. }), "{error:?}");
    assert!(error.detail().unwrap().contains("preferences.language"));
    assert!(core.set_language(Language::English).is_err());
    assert_eq!(
        home.config_text().unwrap(),
        "[preferences]\nlanguage = \"fr\"\n"
    );
}

#[test]
fn the_system_locale_picks_a_language_when_none_is_chosen() {
    assert_eq!(Language::from_locale("it-IT"), Language::Italian);
    assert_eq!(Language::from_locale("it_CH.UTF-8"), Language::Italian);
    assert_eq!(Language::from_locale("en-US"), Language::English);
    assert_eq!(Language::from_locale("de-DE"), Language::English);
    assert_eq!(Language::from_locale(""), Language::English);
}

#[test]
fn languages_have_stable_codes() {
    assert_eq!(Language::Italian.code(), "it");
    assert_eq!(Language::English.code(), "en");
    assert_eq!("it".parse::<Language>().unwrap(), Language::Italian);
    assert!("fr".parse::<Language>().is_err());
}
