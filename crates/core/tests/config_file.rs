//! `~/.mccboard/config.toml`: hand-editable, preserved, never overwritten when corrupt
//! (story 74; ticket #4).

mod common;

use common::{TempHome, UUID_A, UUID_B};
use mccboard_core::{AccountUpdate, AccountUuid, Error};

#[test]
fn the_file_reads_as_documented() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    core.register(&b.dir).unwrap();

    let config: toml::Table = home.config_text().unwrap().parse().unwrap();
    let account = &config["accounts"][UUID_B];
    assert_eq!(account["label"].as_str(), Some("B"));
    assert!(account["color"].as_str().unwrap().starts_with('#'));
    assert_eq!(account["email"].as_str(), Some("b@example.com"));
    let instances = config["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0]["path"].as_str(), Some(b.dir.to_str().unwrap()));
    assert_eq!(instances[0]["account"].as_str(), Some(UUID_B));
}

#[test]
fn a_hand_written_file_is_understood() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    home.write_config(&format!(
        r##"
# My accounts
[accounts.{UUID_B}]
label = "Bee"
color = "#FF8800"

[[instances]]
path = {path:?}
account = "{UUID_B}"
"##,
        path = b.dir.to_str().unwrap()
    ));
    let (core, _env) = home.core();

    let registry = core.registry().unwrap();

    assert_eq!(registry.accounts.len(), 1);
    assert_eq!(registry.accounts[0].label, "Bee");
    assert_eq!(registry.accounts[0].color.as_str(), "#ff8800");
    assert_eq!(registry.accounts[0].email, None);
    assert_eq!(registry.instances.len(), 1);
    assert_eq!(registry.instances[0].config_dir, b.dir);
}

#[test]
fn instances_written_as_an_inline_array_are_understood_and_edited() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let c = home.linked(".claude-c");
    home.write_config(&format!(
        "instances = [{{ path = {b:?} }}, {{ path = {c:?}, note = \"keep me\" }}]\n",
        b = b.dir.to_str().unwrap(),
        c = c.dir.to_str().unwrap(),
    ));
    let (core, _env) = home.core();
    assert_eq!(core.registry().unwrap().instances.len(), 2);

    core.unregister(&b.dir).unwrap();

    let instances = core.registry().unwrap().instances;
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].config_dir, c.dir);
    assert!(home.config_text().unwrap().contains("keep me"));
}

#[test]
fn comments_and_unknown_keys_survive_every_edit() {
    let home = TempHome::new();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    home.write_config(&format!(
        r##"# mccboard settings, edited by hand
future_setting = "keep"

[accounts.{UUID_A}]
label = "A" # the main one
color = "#2563eb"
nickname = "main"

[[instances]]
# the primary
path = {path:?}
account = "{UUID_A}"
pinned = true

[sharing]
gsd-core = "link"
"##,
        path = primary.dir.to_str().unwrap()
    ));
    let (core, _env) = home.core();

    core.register(&b.dir).unwrap();
    core.update_account(
        &AccountUuid::new(UUID_A),
        AccountUpdate {
            color: Some("#16a34a".parse().unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    core.unregister(&b.dir).unwrap();

    let text = home.config_text().unwrap();
    for kept in [
        "# mccboard settings, edited by hand",
        "future_setting = \"keep\"",
        "# the main one",
        "nickname = \"main\"",
        "# the primary",
        "pinned = true",
        "gsd-core = \"link\"",
        "#16a34a",
    ] {
        assert!(text.contains(kept), "lost {kept:?} in:\n{text}");
    }
    assert!(!text.contains(".claude-b"), "{text}");
}

#[test]
fn a_corrupt_file_is_reported_and_never_overwritten() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let corrupt = "[accounts\nlabel = \"oops\n";
    home.write_config(corrupt);
    let (core, _env) = home.core();

    let errors = [
        core.registry().map(|_| ()).unwrap_err(),
        core.scan().map(|_| ()).unwrap_err(),
        core.register(&b.dir).map(|_| ()).unwrap_err(),
        core.unregister(&b.dir).unwrap_err(),
        core.update_account(&AccountUuid::new(UUID_B), AccountUpdate::default())
            .map(|_| ())
            .unwrap_err(),
    ];

    for error in errors {
        assert!(matches!(error, Error::ConfigCorrupt { .. }), "{error:?}");
        assert_eq!(error.code(), "config_corrupt");
        assert!(error.to_string().contains("config.toml"), "{error}");
    }
    assert_eq!(home.config_text().unwrap(), corrupt);
}

#[test]
fn a_file_with_wrong_values_counts_as_corrupt() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let cases = [
        format!("[accounts.{UUID_B}]\nlabel = \"B\"\ncolor = \"blue\"\n"),
        format!("[accounts.{UUID_B}]\ncolor = \"#2563eb\"\n"),
        format!("[accounts.{UUID_B}]\nlabel = \"\"\ncolor = \"#2563eb\"\n"),
        "instances = \"not a list\"\n".to_owned(),
        "[[instances]]\naccount = \"x\"\n".to_owned(),
        "[[instances]]\npath = \"relative/dir\"\n".to_owned(),
    ];
    for corrupt in cases {
        home.write_config(&corrupt);
        let (core, _env) = home.core();

        let error = core.register(&b.dir).map(|_| ()).unwrap_err();

        assert_eq!(error.code(), "config_corrupt", "{corrupt}");
        assert_eq!(home.config_text().unwrap(), corrupt);
    }
}

#[test]
fn an_empty_file_is_an_empty_configuration() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    home.write_config("");
    let (core, _env) = home.core();

    assert!(core.registry().unwrap().instances.is_empty());
    core.register(&b.dir).unwrap();
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[test]
fn an_instance_registered_under_an_account_missing_from_the_file_still_lists() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    home.write_config(&format!(
        "[[instances]]\npath = {path:?}\naccount = \"{UUID_B}\"\n",
        path = b.dir.to_str().unwrap()
    ));
    let (core, _env) = home.core();

    let registry = core.registry().unwrap();

    assert!(registry.accounts.is_empty());
    assert_eq!(
        registry.instances[0].registered_account,
        Some(AccountUuid::new(UUID_B))
    );

    // Registering again fills in the missing account.
    let registration = core.register(&b.dir).unwrap();
    assert_eq!(registration.new_account.unwrap().label, "B");
}
