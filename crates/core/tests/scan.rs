//! Scanning a home for instance candidates (spec stories 6–7, 9–10, 12).

mod common;

use std::fs;

use common::{FIXTURE_REFRESH, FIXTURE_TOKEN, TempHome, now};
use jiff::SignedDuration;
use mccboard_core::{AccountUuid, Error, InstanceKind, LoginState, ObservedAccount, Warning};

const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
const UUID_B: &str = "22222222-2222-4222-8222-222222222222";

fn hours(n: i64) -> SignedDuration {
    SignedDuration::from_hours(n)
}

#[test]
fn an_empty_home_has_no_candidates() {
    let home = TempHome::new();
    let (core, _env) = home.core();

    assert_eq!(core.scan().unwrap(), vec![]);
}

#[test]
fn finds_the_primary_and_linked_instances_primary_first() {
    let home = TempHome::new();
    home.linked(".claude-zeta");
    home.linked(".claude-b");
    home.primary();
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();

    let found: Vec<_> = candidates
        .iter()
        .map(|c| (c.config_dir.clone(), c.kind))
        .collect();
    assert_eq!(
        found,
        vec![
            (home.path().join(".claude"), InstanceKind::Primary),
            (home.path().join(".claude-b"), InstanceKind::Linked),
            (home.path().join(".claude-zeta"), InstanceKind::Linked),
        ]
    );
}

#[test]
fn ignores_files_and_folders_that_are_not_config_directories() {
    let home = TempHome::new();
    home.write(".claude.json", "{}");
    home.write(".claude-notes", "a file, not a directory");
    home.write(".claude.json.backup", "{}");
    fs::create_dir(home.path().join(".claudette")).unwrap();
    fs::create_dir(home.path().join(".claude-")).unwrap();
    fs::create_dir(home.path().join("claude-b")).unwrap();
    let (core, _env) = home.core();

    assert_eq!(core.scan().unwrap(), vec![]);
}

#[test]
fn a_linked_instance_without_a_primary_is_still_proposed() {
    let home = TempHome::new();
    home.linked(".claude-b");
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, InstanceKind::Linked);
}

#[test]
fn the_observed_account_comes_from_each_instances_global_config() {
    let home = TempHome::new();
    // The primary's global config lives next to the config directory, not inside it.
    home.primary().logged_in_as(UUID_A, "a@example.com");
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com");
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();

    assert_eq!(
        candidates[0].observation.account,
        Some(ObservedAccount {
            uuid: AccountUuid::new(UUID_A),
            email: Some("a@example.com".into()),
        })
    );
    assert_eq!(
        candidates[1].observation.account,
        Some(ObservedAccount {
            uuid: AccountUuid::new(UUID_B),
            email: Some("b@example.com".into()),
        })
    );
}

#[test]
fn the_primarys_global_config_is_not_read_from_inside_its_directory() {
    let home = TempHome::new();
    let primary = home.primary();
    fs::write(
        primary.dir.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"33333333-3333-4333-8333-333333333333"}}"#,
    )
    .unwrap();
    let (core, _env) = home.core();

    assert_eq!(core.scan().unwrap()[0].observation.account, None);
}

#[test]
fn an_instance_with_no_login_has_no_account_and_is_not_logged_in() {
    let home = TempHome::new();
    home.primary();
    home.linked(".claude-b")
        .global_config(serde_json::json!({"numStartups": 1}));
    let (core, _env) = home.core();

    for candidate in core.scan().unwrap() {
        assert_eq!(candidate.observation.account, None);
        assert_eq!(candidate.observation.login, LoginState::NotLoggedIn);
        assert_eq!(candidate.observation.warnings, vec![]);
    }
}

#[test]
fn an_account_without_an_email_is_still_observed() {
    let home = TempHome::new();
    home.primary()
        .global_config(serde_json::json!({"oauthAccount": {"accountUuid": UUID_A}}));
    let (core, _env) = home.core();

    assert_eq!(
        core.scan().unwrap()[0].observation.account,
        Some(ObservedAccount {
            uuid: AccountUuid::new(UUID_A),
            email: None,
        })
    );
}

#[test]
fn a_token_that_has_not_expired_reads_as_logged_in() {
    let home = TempHome::new();
    let expires_at = now() + hours(5);
    home.primary().credentials_expiring(expires_at);
    let (core, _env) = home.core();

    assert_eq!(
        core.scan().unwrap()[0].observation.login,
        LoginState::LoggedIn { expires_at }
    );
}

#[test]
fn a_token_past_its_expiry_reads_as_expired() {
    let home = TempHome::new();
    let expires_at = now() - hours(1);
    home.linked(".claude-b").credentials_expiring(expires_at);
    let (core, _env) = home.core();

    assert_eq!(
        core.scan().unwrap()[0].observation.login,
        LoginState::Expired {
            expired_at: expires_at
        }
    );
}

#[test]
fn login_state_follows_the_clock() {
    let home = TempHome::new();
    let expires_at = now() + hours(2);
    home.primary().credentials_expiring(expires_at);
    let (core, env) = home.core();
    assert!(matches!(
        core.scan().unwrap()[0].observation.login,
        LoginState::LoggedIn { .. }
    ));

    env.advance(hours(2));

    assert_eq!(
        core.scan().unwrap()[0].observation.login,
        LoginState::Expired {
            expired_at: expires_at
        }
    );
}

#[test]
fn a_token_cleared_by_claude_code_reads_as_not_logged_in() {
    // After a dead refresh token Claude Code writes empty tokens and `expiresAt: 0`: that is a
    // logout, not an expired token Claude Code will renew.
    let home = TempHome::new();
    home.primary()
        .raw_credentials(r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0}}"#);
    let (core, _env) = home.core();

    assert_eq!(
        core.scan().unwrap()[0].observation.login,
        LoginState::NotLoggedIn
    );
}

#[test]
fn credentials_without_an_access_token_read_as_not_logged_in() {
    // Only a token, valid or expired, is a login: an expiry alone says nothing.
    let expires_at = (now() + hours(5)).as_millisecond();
    for credentials in [
        format!(r#"{{"claudeAiOauth":{{"expiresAt":{expires_at}}}}}"#),
        format!(r#"{{"claudeAiOauth":{{"accessToken":null,"expiresAt":{expires_at}}}}}"#),
    ] {
        let home = TempHome::new();
        home.primary().raw_credentials(&credentials);
        let (core, _env) = home.core();

        let observation = &core.scan().unwrap()[0].observation;

        assert_eq!(observation.login, LoginState::NotLoggedIn, "{credentials}");
        assert!(observation.warnings.is_empty(), "{credentials}");
    }
}

#[test]
fn an_access_token_that_is_not_a_string_is_unreadable() {
    let expires_at = (now() + hours(5)).as_millisecond();
    let home = TempHome::new();
    home.primary().raw_credentials(&format!(
        r#"{{"claudeAiOauth":{{"accessToken":42,"expiresAt":{expires_at}}}}}"#
    ));
    let (core, _env) = home.core();

    let observation = &core.scan().unwrap()[0].observation;

    assert_eq!(observation.login, LoginState::Unknown);
    assert_eq!(observation.warnings, vec![Warning::CredentialsUnreadable]);
}

#[test]
fn credentials_without_a_claude_ai_login_read_as_not_logged_in() {
    let home = TempHome::new();
    home.primary()
        .raw_credentials(r#"{"mcpOAuth":{"some-server":{"accessToken":"x"}}}"#);
    let (core, _env) = home.core();

    assert_eq!(
        core.scan().unwrap()[0].observation.login,
        LoginState::NotLoggedIn
    );
}

#[test]
fn unreadable_files_are_reported_not_fatal() {
    let home = TempHome::new();
    home.primary();
    home.write(".claude.json", "{ not json");
    home.write(".claude/.credentials.json", "{ not json either");
    let (core, _env) = home.core();

    let candidate = &core.scan().unwrap()[0];

    assert_eq!(candidate.observation.account, None);
    assert_eq!(candidate.observation.login, LoginState::Unknown);
    assert_eq!(
        candidate.observation.warnings,
        vec![
            Warning::GlobalConfigUnreadable,
            Warning::CredentialsUnreadable
        ]
    );
}

#[test]
fn observe_reads_one_config_directory() {
    let home = TempHome::new();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() + hours(1));
    let (core, _env) = home.core();

    let observation = core.observe(&b.dir);

    assert_eq!(
        observation.account.map(|a| a.uuid),
        Some(AccountUuid::new(UUID_B))
    );
    assert!(matches!(observation.login, LoginState::LoggedIn { .. }));
}

#[test]
fn scanning_changes_nothing_in_the_home() {
    let home = TempHome::new();
    home.primary()
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(1));
    home.linked(".claude-b");
    let before = tree(home.path());
    let (core, _env) = home.core();

    core.scan().unwrap();

    assert_eq!(tree(home.path()), before);
}

#[test]
fn a_missing_home_is_an_error() {
    let home = TempHome::new();
    let missing = home.path().join("nobody");
    let core = mccboard_core::Core::new(
        &missing,
        mccboard_core::testing::FakeEnvironment::new(now()),
    );

    match core.scan() {
        Err(Error::HomeUnreadable { path, .. }) => assert_eq!(path, missing),
        other => panic!("expected HomeUnreadable, got {other:?}"),
    }
}

#[test]
fn no_token_appears_in_anything_scan_produces() {
    let home = TempHome::new();
    home.primary()
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring(now() + hours(1));
    home.linked(".claude-b")
        .logged_in_as(UUID_B, "b@example.com")
        .credentials_expiring(now() - hours(1));
    // A malformed file whose only string values are tokens: parse errors must not echo them.
    home.linked(".claude-c").raw_credentials(&format!(
        r#"{{"claudeAiOauth":{{"accessToken":"{FIXTURE_TOKEN}","expiresAt":"{FIXTURE_REFRESH}"}}"#
    ));
    home.linked(".claude-d").raw_credentials(&format!(
        r#"{{"claudeAiOauth":{{"expiresAt":"{FIXTURE_TOKEN}"}}}}"#
    ));
    // An access token of the wrong type, and an emptied one next to a refresh token.
    home.linked(".claude-e").raw_credentials(&format!(
        r#"{{"claudeAiOauth":{{"accessToken":["{FIXTURE_TOKEN}"],"expiresAt":0}}}}"#
    ));
    home.linked(".claude-f").raw_credentials(&format!(
        r#"{{"claudeAiOauth":{{"accessToken":"","refreshToken":"{FIXTURE_REFRESH}","expiresAt":0}}}}"#
    ));
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();
    let outputs = [
        format!("{candidates:?}"),
        serde_json::to_string(&candidates).unwrap(),
    ];

    for output in outputs {
        assert!(!output.contains(FIXTURE_TOKEN), "token leaked: {output}");
        assert!(!output.contains(FIXTURE_REFRESH), "token leaked: {output}");
        assert!(!output.contains("FIXTURE"), "token leaked: {output}");
    }
    assert_eq!(candidates[2].observation.login, LoginState::Unknown);
    assert_eq!(candidates[3].observation.login, LoginState::Unknown);
    assert_eq!(candidates[4].observation.login, LoginState::Unknown);
    assert_eq!(candidates[5].observation.login, LoginState::NotLoggedIn);
}

#[test]
fn candidates_serialize_to_a_stable_json_shape() {
    let home = TempHome::new();
    home.primary()
        .logged_in_as(UUID_A, "a@example.com")
        .credentials_expiring("2026-10-15T17:00:00Z".parse().unwrap());
    home.linked(".claude-b");
    let (core, _env) = home.core();

    let json = serde_json::to_value(core.scan().unwrap()).unwrap();

    assert_eq!(
        json,
        serde_json::json!([
            {
                "config_dir": home.path().join(".claude"),
                "kind": "primary",
                "registered": false,
                "account": { "uuid": UUID_A, "email": "a@example.com" },
                "login": { "state": "logged_in", "expires_at": "2026-10-15T17:00:00Z" },
                "warnings": [],
            },
            {
                "config_dir": home.path().join(".claude-b"),
                "kind": "linked",
                "registered": false,
                "account": null,
                "login": { "state": "not_logged_in" },
                "warnings": [],
            },
        ])
    );
}

/// Every path under `root` with its file contents (directories as `None`).
fn tree(root: &std::path::Path) -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                entries.push((path.clone(), None));
                stack.push(path);
            } else {
                entries.push((path.clone(), Some(fs::read(&path).unwrap())));
            }
        }
    }
    entries.sort();
    entries
}

#[cfg(unix)]
#[test]
fn a_config_directory_reached_through_a_link_is_proposed() {
    let home = TempHome::new();
    let elsewhere = home.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, home.path().join(".claude-b")).unwrap();
    let (core, _env) = home.core();

    let candidates = core.scan().unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].config_dir, home.path().join(".claude-b"));
}

#[test]
fn errors_carry_a_stable_code_and_no_file_contents() {
    let home = TempHome::new();
    let core = mccboard_core::Core::new(
        home.path().join("nobody"),
        mccboard_core::testing::FakeEnvironment::new(now()),
    );

    let error = core.scan().unwrap_err();

    assert_eq!(error.code(), "home_unreadable");
    assert!(error.to_string().contains("nobody"));
}
