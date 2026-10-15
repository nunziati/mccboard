//! The usage endpoint: the real HTTP call and the reading of its reply.
//!
//! `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer <token>` and
//! `anthropic-beta: oauth-2025-04-20`. The reply is read leniently: windows come from the
//! `limits[]` rows (kinds `session`, `weekly_all`, `weekly_scoped`), with the flat `five_hour`
//! and `seven_day` keys filling any gap. Bodies are never shown or stored: they could echo
//! anything, including what was sent.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value};

use super::token::AccessToken;
use super::{ModelWindow, UsageWindow};
use crate::EndpointReply;

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
/// Anthropic's edge may reject generic HTTP client user agents; Claude Code's own form passes.
const USER_AGENT: &str = concat!("claude-code/2.1.300 mccboard/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);
/// A usage reply is a few KB; anything far larger isn't one.
const MAX_BODY: u64 = 1 << 20;

/// Makes the call. Errors carry a fixed description of their kind, never the client's own
/// message, which could quote the request.
pub(crate) fn call(token: &AccessToken) -> EndpointReply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        // A redirect could carry the token elsewhere; it reads as a failed status instead.
        .max_redirects(0)
        .user_agent(USER_AGENT)
        .build()
        .into();
    let response = agent
        .get(URL)
        .header("Authorization", format!("Bearer {}", token.expose()))
        .header("anthropic-beta", BETA)
        .header("Content-Type", "application/json")
        .call();
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            return EndpointReply::Unreachable {
                reason: describe(&error).to_owned(),
            };
        }
    };
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|secs| SignedDuration::from_secs(secs.into()));
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .unwrap_or_default();
    EndpointReply::Http {
        status,
        retry_after,
        body,
    }
}

fn describe(error: &ureq::Error) -> &'static str {
    match error {
        ureq::Error::Timeout(_) => "timeout",
        ureq::Error::HostNotFound => "host not found",
        ureq::Error::ConnectionFailed => "connection failed",
        ureq::Error::Io(_) => "network error",
        ureq::Error::InvalidProxyUrl => "invalid proxy setting",
        ureq::Error::Tls(_) | ureq::Error::Pem(_) | ureq::Error::Rustls(_) => "TLS error",
        _ => "HTTP client error",
    }
}

/// The windows read from a successful reply.
pub(crate) struct Windows {
    pub session: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    pub models: Vec<ModelWindow>,
}

/// Reads a 200 reply's body. `None` when it isn't a usage reply: not JSON, not an object, a
/// `limits` that isn't a list, or no window at all.
pub(crate) fn parse(body: &str) -> Option<Windows> {
    let value: Value = serde_json::from_str(body).ok()?;
    let reply = value.as_object()?;
    let limits: Vec<&Map<String, Value>> = match reply.get("limits") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(rows)) => rows.iter().filter_map(Value::as_object).collect(),
        Some(_) => return None,
    };
    let kind = |row: &&Map<String, Value>, kind: &str| {
        row.get("kind").and_then(Value::as_str) == Some(kind)
    };
    let row = |wanted: &str| {
        limits
            .iter()
            .filter(|row| kind(row, wanted))
            .find_map(|row| window(row, "percent"))
    };
    let flat = |key: &str| {
        reply
            .get(key)
            .and_then(Value::as_object)
            .and_then(|w| window(w, "utilization"))
    };

    let session = row("session").or_else(|| flat("five_hour"));
    let weekly = row("weekly_all").or_else(|| flat("seven_day"));
    let models: Vec<ModelWindow> = limits
        .iter()
        .filter(|row| kind(row, "weekly_scoped"))
        .filter_map(|row| {
            let model = row
                .get("scope")?
                .get("model")?
                .get("display_name")?
                .as_str()?
                .trim();
            let window = window(row, "percent")?;
            (!model.is_empty()).then(|| ModelWindow {
                model: model.to_owned(),
                utilization: window.utilization,
                resets_at: window.resets_at,
            })
        })
        .collect();

    if session.is_none() && weekly.is_none() && models.is_empty() {
        return None;
    }
    Some(Windows {
        session,
        weekly,
        models,
    })
}

/// A window from an object with a 0–100 number under `key` and an RFC 3339 `resets_at`.
/// A missing or null number means the window has no data.
fn window(object: &Map<String, Value>, key: &str) -> Option<UsageWindow> {
    let utilization = object.get(key)?.as_f64()?.clamp(0.0, 100.0);
    let resets_at = object
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Timestamp>().ok());
    Some(UsageWindow {
        utilization,
        resets_at,
    })
}
