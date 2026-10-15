//! Accounts: identity by UUID, suggested and edited labels and colors (stories 1–5).

mod common;

use common::{TempHome, UUID_A, UUID_B, UUID_C};
use mccboard_core::{AccountUpdate, AccountUuid, Color, Error};

fn color(hex: &str) -> Color {
    hex.parse().unwrap()
}

#[test]
fn a_new_account_gets_a_suggested_label_and_color() {
    let home = TempHome::new();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();

    let account = core.register(&primary.dir).unwrap().new_account.unwrap();

    assert_eq!(account.uuid, AccountUuid::new(UUID_A));
    assert_eq!(account.label, "A");
    assert_eq!(account.email.as_deref(), Some("a@example.com"));
    assert_eq!(core.registry().unwrap().accounts, vec![account]);
}

#[test]
fn a_linked_instance_suggests_its_folder_suffix_as_label() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    home.linked(".claude-work")
        .logged_in_as(UUID_C, "c@example.com");
    let (core, _env) = home.core();

    let labels: Vec<_> = [".claude", ".claude-b", ".claude-work"]
        .into_iter()
        .map(|dir| {
            core.register(&home.path().join(dir))
                .unwrap()
                .new_account
                .unwrap()
                .label
        })
        .collect();

    assert_eq!(labels, vec!["A", "B", "work"]);
}

#[test]
fn suggestions_avoid_labels_and_colors_already_taken() {
    let home = TempHome::new();
    // `.claude-a` would suggest "A", which the primary's account already has.
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-a")
        .logged_in_as(UUID_B, "b@example.com");
    home.linked(".claude-x")
        .logged_in_as(UUID_C, "c@example.com");
    let (core, _env) = home.core();

    let a = core.register(&home.path().join(".claude")).unwrap();
    let b = core.register(&home.path().join(".claude-a")).unwrap();
    let c = core.register(&home.path().join(".claude-x")).unwrap();

    let (a, b, c) = (
        a.new_account.unwrap(),
        b.new_account.unwrap(),
        c.new_account.unwrap(),
    );
    assert_eq!(a.label, "A");
    assert_eq!(b.label, "B");
    assert_eq!(c.label, "X");
    assert_ne!(a.color, b.color);
    assert_ne!(b.color, c.color);
    assert_ne!(a.color, c.color);
}

#[test]
fn label_color_and_launcher_can_be_changed_and_persist() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    let uuid = AccountUuid::new(UUID_B);

    let updated = core
        .update_account(
            &uuid,
            AccountUpdate {
                label: Some("Work".into()),
                color: Some(color("#0D9488")),
                launcher: Some(Some("cw".into())),
            },
        )
        .unwrap();

    assert_eq!(updated.label, "Work");
    assert_eq!(updated.color.as_str(), "#0d9488");
    assert_eq!(updated.launcher_name(), "cw");
    let (restarted, _env) = home.core();
    assert_eq!(restarted.registry().unwrap().accounts, vec![updated]);
}

#[test]
fn the_launcher_name_follows_the_label_unless_overridden() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    let uuid = AccountUuid::new(UUID_B);

    let account = core.register(&b.dir).unwrap().new_account.unwrap();
    assert_eq!(account.launcher, None);
    assert_eq!(account.launcher_name(), "claude-b");

    let renamed = core
        .update_account(
            &uuid,
            AccountUpdate {
                label: Some("Personal Max".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(renamed.launcher_name(), "claude-personal-max");

    let overridden = core
        .update_account(
            &uuid,
            AccountUpdate {
                launcher: Some(Some("cb".into())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(overridden.launcher_name(), "cb");

    let reset = core
        .update_account(
            &uuid,
            AccountUpdate {
                launcher: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(reset.launcher, None);
    assert_eq!(reset.launcher_name(), "claude-personal-max");
}

#[test]
fn an_update_that_changes_nothing_keeps_the_other_fields() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    let before = core.register(&b.dir).unwrap().new_account.unwrap();

    let after = core
        .update_account(&before.uuid, AccountUpdate::default())
        .unwrap();

    assert_eq!(after, before);
}

#[test]
fn labels_are_unique_ignoring_case() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    core.register(&home.path().join(".claude")).unwrap();
    core.register(&home.path().join(".claude-b")).unwrap();
    let config = home.config_text().unwrap();

    let error = core
        .update_account(
            &AccountUuid::new(UUID_B),
            AccountUpdate {
                label: Some("a".into()),
                ..Default::default()
            },
        )
        .unwrap_err();

    assert!(matches!(error, Error::LabelTaken { .. }), "{error:?}");
    assert_eq!(error.code(), "label_taken");
    assert_eq!(home.config_text().unwrap(), config);
}

#[test]
fn an_account_keeps_its_own_label_when_renamed_to_another_case() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();

    let updated = core
        .update_account(
            &AccountUuid::new(UUID_B),
            AccountUpdate {
                label: Some("b".into()),
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(updated.label, "b");
}

#[test]
fn invalid_labels_colors_and_launchers_are_refused() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    let uuid = AccountUuid::new(UUID_B);

    for label in ["", "   ", "x".repeat(41).as_str()] {
        let error = core
            .update_account(
                &uuid,
                AccountUpdate {
                    label: Some(label.into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_label", "{label:?}");
    }
    for hex in ["blue", "#12345", "#12345g", "123456"] {
        let error = hex.parse::<Color>().unwrap_err();
        assert_eq!(error.code(), "invalid_color", "{hex:?}");
    }
    for name in ["", "claude", "has space", "a/b", "-x", "a;b"] {
        let error = core
            .update_account(
                &uuid,
                AccountUpdate {
                    launcher: Some(Some(name.into())),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "invalid_launcher", "{name:?}");
    }
}

#[test]
fn a_label_that_makes_no_launcher_name_needs_an_override() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    let account = core.register(&b.dir).unwrap().new_account.unwrap();

    let renamed = core
        .update_account(
            &account.uuid,
            AccountUpdate {
                label: Some("Ü ★".into()),
                ..Default::default()
            },
        )
        .unwrap();

    // Non-ASCII is dropped from the derived name; nothing left falls back to the UUID prefix.
    assert_eq!(renamed.launcher_name(), "claude-22222222");
}

#[test]
fn updating_an_unknown_account_is_an_error() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    let error = core
        .update_account(&AccountUuid::new(UUID_C), AccountUpdate::default())
        .unwrap_err();

    assert!(matches!(error, Error::AccountNotFound { .. }), "{error:?}");
    assert_eq!(error.code(), "account_not_found");
    assert_eq!(home.config_text(), None);
}

#[test]
fn accounts_are_listed_by_label() {
    let home = TempHome::new();
    home.linked(".claude-z")
        .logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();
    core.register(&home.path().join(".claude-z")).unwrap();
    core.register(&home.path().join(".claude-b")).unwrap();

    let labels: Vec<_> = core
        .registry()
        .unwrap()
        .accounts
        .into_iter()
        .map(|a| a.label)
        .collect();

    assert_eq!(labels, vec!["B", "Z"]);
}
