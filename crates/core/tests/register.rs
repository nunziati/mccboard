//! Registering and unregistering instances (spec stories 1, 6–12, 74; ticket #4).

mod common;

use std::fs;

use common::{FIXTURE_REFRESH, FIXTURE_TOKEN, TempHome, UUID_A, UUID_B, now};
use jiff::SignedDuration;
use mccboard_core::{AccountStatus, AccountUuid, Error, InstanceKind, LoginState, Warning};

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

#[test]
fn a_fresh_home_has_nothing_registered_and_reading_writes_nothing() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();

    let registry = core.registry().unwrap();

    assert!(registry.accounts.is_empty());
    assert!(registry.instances.is_empty());
    assert_eq!(home.config_text(), None);
}

#[test]
fn the_scan_only_proposes_and_registers_nothing() {
    let home = TempHome::new();
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();

    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|c| !c.registered));
    assert!(core.registry().unwrap().instances.is_empty());
    assert_eq!(home.config_text(), None);
}

#[test]
fn registering_a_candidate_persists_it_across_restarts() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(5));
    let (core, _env) = home.core();
    let candidate = core.scan().unwrap().remove(0);

    let registration = core.register(&candidate.config_dir).unwrap();

    assert_eq!(registration.instance.config_dir, primary.dir);
    assert_eq!(registration.instance.kind, InstanceKind::Primary);
    assert_eq!(
        registration.instance.registered_account,
        Some(AccountUuid::new(UUID_A))
    );
    assert_eq!(
        registration.instance.account_status,
        AccountStatus::Unchanged
    );

    // A new core over the same home is mccboard after a restart.
    let (restarted, _env) = home.core();
    let registry = restarted.registry().unwrap();
    assert_eq!(registry.instances.len(), 1);
    assert_eq!(registry.instances[0].config_dir, primary.dir);
    assert!(matches!(
        registry.instances[0].observation.login,
        LoginState::LoggedIn { .. }
    ));
    assert_eq!(registry.accounts.len(), 1);
    assert_eq!(registry.accounts[0].uuid, AccountUuid::new(UUID_A));
    assert!(restarted.scan().unwrap()[0].registered);
}

#[test]
fn instances_of_the_same_account_group_under_one_account() {
    let home = TempHome::new();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    let other = home.linked(".claude-a2");
    other.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();

    let first = core.register(&primary.dir).unwrap();
    let second = core.register(&other.dir).unwrap();

    assert!(first.new_account.is_some());
    assert_eq!(second.new_account, None);
    let registry = core.registry().unwrap();
    assert_eq!(registry.accounts.len(), 1);
    assert_eq!(registry.instances.len(), 2);
    assert!(
        registry
            .instances
            .iter()
            .all(|i| i.registered_account == Some(AccountUuid::new(UUID_A)))
    );
}

#[test]
fn registered_instances_are_listed_primary_first_then_by_path() {
    let home = TempHome::new();
    let zeta = home.linked(".claude-zeta");
    let b = home.linked(".claude-b");
    let primary = home.primary();
    let (core, _env) = home.core();
    for dir in [&zeta.dir, &primary.dir, &b.dir] {
        core.register(dir).unwrap();
    }

    let dirs: Vec<_> = core
        .registry()
        .unwrap()
        .instances
        .into_iter()
        .map(|i| i.config_dir)
        .collect();

    assert_eq!(dirs, vec![primary.dir, b.dir, zeta.dir]);
}

#[test]
fn an_instance_without_a_login_can_be_registered() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();

    let registration = core.register(&b.dir).unwrap();

    assert_eq!(registration.instance.registered_account, None);
    assert_eq!(registration.new_account, None);
    assert_eq!(
        registration.instance.observation.login,
        LoginState::NotLoggedIn
    );
    assert_eq!(
        registration.instance.account_status,
        AccountStatus::NotObserved
    );
    assert!(core.registry().unwrap().accounts.is_empty());
}

#[test]
fn an_instance_at_any_path_can_be_registered() {
    let home = TempHome::new();
    let odd = home.path().join("work").join("claude config");
    fs::create_dir_all(&odd).unwrap();
    fs::write(
        odd.join(".claude.json"),
        format!(r#"{{"oauthAccount":{{"accountUuid":"{UUID_B}"}}}}"#),
    )
    .unwrap();
    let (core, _env) = home.core();

    let registration = core.register(&odd).unwrap();

    assert_eq!(registration.instance.config_dir, odd);
    assert_eq!(registration.instance.kind, InstanceKind::Linked);
    assert_eq!(
        registration.instance.registered_account,
        Some(AccountUuid::new(UUID_B))
    );
}

#[test]
fn a_path_starting_with_a_tilde_is_inside_the_home() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();

    let registration = core.register("~/.claude-b".as_ref()).unwrap();

    assert_eq!(registration.instance.config_dir, b.dir);
}

#[test]
fn paths_are_normalised_so_the_primary_is_recognised_and_not_registered_twice() {
    let home = TempHome::new();
    let primary = home.primary();
    home.linked(".claude-b");
    let (core, _env) = home.core();

    let roundabout = home
        .path()
        .join(".claude-b")
        .join("..")
        .join(".")
        .join(".claude");
    let registration = core.register(&roundabout).unwrap();
    core.register(&primary.dir).unwrap();

    assert_eq!(registration.instance.config_dir, primary.dir);
    assert_eq!(registration.instance.kind, InstanceKind::Primary);
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[cfg(windows)]
#[test]
fn on_windows_paths_differing_only_in_case_are_the_same_instance() {
    let home = TempHome::new();
    let primary = home.primary();
    let (core, _env) = home.core();

    let shouted = std::path::PathBuf::from(primary.dir.to_string_lossy().to_uppercase());
    let registration = core.register(&shouted).unwrap();
    core.register(&primary.dir).unwrap();

    assert_eq!(registration.instance.kind, InstanceKind::Primary);
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[test]
fn registering_twice_is_harmless() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    core.register(&b.dir).unwrap();
    let config = home.config_text().unwrap();
    let again = core.register(&b.dir).unwrap();

    assert_eq!(again.new_account, None);
    assert_eq!(home.config_text().unwrap(), config);
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[test]
fn a_relative_path_is_refused() {
    let home = TempHome::new();
    home.linked(".claude-b");
    let (core, _env) = home.core();

    let error = core.register(".claude-b".as_ref()).unwrap_err();

    assert!(matches!(error, Error::RelativePath { .. }), "{error:?}");
    assert_eq!(error.code(), "relative_path");
    assert_eq!(home.config_text(), None);
}

#[test]
fn a_folder_that_does_not_exist_is_refused() {
    let home = TempHome::new();
    home.write(".claude-file", "not a folder");
    let (core, _env) = home.core();

    for path in [
        home.path().join(".claude-nowhere"),
        home.path().join(".claude-file"),
    ] {
        let error = core.register(&path).unwrap_err();
        assert!(matches!(error, Error::NotADirectory { .. }), "{error:?}");
        assert_eq!(error.code(), "not_a_directory");
    }
    assert_eq!(home.config_text(), None);
}

#[test]
fn unregistering_forgets_the_instance_and_touches_nothing_on_disk() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() + hours(1));
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    let credentials_before = fs::read(b.dir.join(".credentials.json")).unwrap();

    core.unregister(&b.dir).unwrap();

    let registry = core.registry().unwrap();
    assert!(registry.instances.is_empty());
    // The account keeps its label and color for when an instance comes back.
    assert_eq!(registry.accounts.len(), 1);
    assert!(b.dir.is_dir());
    assert_eq!(
        fs::read(b.dir.join(".credentials.json")).unwrap(),
        credentials_before
    );
    assert!(!core.scan().unwrap()[0].registered);
}

#[test]
fn unregistering_accepts_the_path_in_any_spelling() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();

    core.unregister("~/.claude-b/".as_ref()).unwrap();

    assert!(core.registry().unwrap().instances.is_empty());
}

#[test]
fn unregistering_an_unknown_instance_is_an_error() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();

    let error = core.unregister(&b.dir).unwrap_err();

    assert!(matches!(error, Error::NotRegistered { .. }), "{error:?}");
    assert_eq!(error.code(), "not_registered");
}

#[test]
fn a_deleted_folder_is_flagged_and_can_still_be_unregistered() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    fs::remove_dir_all(&b.dir).unwrap();

    let listed = core.registry().unwrap().instances.remove(0);
    assert!(
        listed
            .observation
            .warnings
            .contains(&Warning::ConfigDirMissing)
    );

    core.unregister(&b.dir).unwrap();
    assert!(core.registry().unwrap().instances.is_empty());
}

#[test]
fn a_different_observed_account_is_reported_as_changed() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();

    b.logged_in_as(UUID_B, "b@example.com");
    let instance = core.registry().unwrap().instances.remove(0);

    assert_eq!(
        instance.account_status,
        AccountStatus::Changed {
            registered: AccountUuid::new(UUID_A)
        }
    );
    assert_eq!(instance.registered_account, Some(AccountUuid::new(UUID_A)));
    assert_eq!(
        instance.observation.account.unwrap().uuid,
        AccountUuid::new(UUID_B)
    );
}

#[test]
fn registering_again_accepts_the_new_account() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    b.logged_in_as(UUID_B, "b@example.com");

    let registration = core.register(&b.dir).unwrap();

    assert_eq!(
        registration.instance.account_status,
        AccountStatus::Unchanged
    );
    assert_eq!(
        registration.instance.registered_account,
        Some(AccountUuid::new(UUID_B))
    );
    assert_eq!(
        registration.new_account.map(|a| a.uuid),
        Some(AccountUuid::new(UUID_B))
    );
    assert_eq!(core.registry().unwrap().instances.len(), 1);
}

#[test]
fn logging_out_is_not_an_account_change() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();

    fs::remove_file(&b.global_config).unwrap();
    let instance = core.registry().unwrap().instances.remove(0);

    assert_eq!(instance.account_status, AccountStatus::NotObserved);
    assert_eq!(instance.registered_account, Some(AccountUuid::new(UUID_A)));
}

#[test]
fn a_login_after_registration_is_unrecorded_until_registered_again() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();

    b.logged_in_as(UUID_B, "b@example.com");
    let instance = core.registry().unwrap().instances.remove(0);
    assert_eq!(instance.account_status, AccountStatus::Unrecorded);

    let registration = core.register(&b.dir).unwrap();
    assert_eq!(
        registration.instance.account_status,
        AccountStatus::Unchanged
    );
    assert_eq!(core.registry().unwrap().accounts.len(), 1);
}

#[test]
fn an_expired_login_keeps_the_instance_and_its_account() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() + hours(1));
    let (core, env) = home.core();
    core.register(&b.dir).unwrap();

    env.advance(hours(2));
    let instance = core.registry().unwrap().instances.remove(0);

    assert!(matches!(
        instance.observation.login,
        LoginState::Expired { .. }
    ));
    assert_eq!(instance.account_status, AccountStatus::Unchanged);
}

#[test]
fn no_token_reaches_the_config_file_or_the_registry() {
    let home = TempHome::new();
    let primary = home.primary();
    primary
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(1));
    let (core, _env) = home.core();

    core.register(&primary.dir).unwrap();
    let registry = core.registry().unwrap();

    let config = home.config_text().unwrap();
    let json = serde_json::to_string(&registry).unwrap();
    let debug = format!("{registry:?}");
    for text in [&config, &json, &debug] {
        assert!(!text.contains(FIXTURE_TOKEN));
        assert!(!text.contains(FIXTURE_REFRESH));
    }
}

#[test]
fn the_registry_serializes_to_a_stable_json_shape() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_A, "a@example.com");
    let (core, _env) = home.core();
    core.register(&b.dir).unwrap();
    b.logged_in_as(UUID_B, "b@example.com");

    let json = serde_json::to_value(core.registry().unwrap()).unwrap();

    assert_eq!(
        json,
        serde_json::json!({
            "accounts": [{
                "uuid": UUID_A,
                "label": "B",
                "color": "#2563eb",
                "email": "a@example.com",
                "launcher": null,
                "launcher_name": "claude-b",
            }],
            "instances": [{
                "config_dir": b.dir,
                "kind": "linked",
                "registered_account": UUID_A,
                "account": { "uuid": UUID_B, "email": "b@example.com" },
                "login": { "state": "not_logged_in" },
                "warnings": [],
                "account_status": { "status": "changed", "registered": UUID_A },
            }],
        })
    );
}
