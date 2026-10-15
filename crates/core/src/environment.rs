use jiff::{SignedDuration, Timestamp};

use crate::AccessToken;

/// The port through which the core reaches everything outside the filesystem.
///
/// One real implementation per OS ([`SystemEnvironment`]) and one fake for tests
/// (`testing::FakeEnvironment`). Later features add to it: running `claude` commands, listing
/// processes, checking Windows Developer Mode.
pub trait Environment: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;

    /// Calls the usage endpoint (`GET https://api.anthropic.com/api/oauth/usage`) with
    /// `token`. The core calls it at most once per account per rate-limit interval, and never
    /// with an expired token. An implementation must not log, store or echo the token.
    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply;
}

/// What the usage endpoint answered. The core reads it; the port only carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointReply {
    /// An HTTP reply, whatever its status. `retry_after` is the `Retry-After` header in
    /// seconds, when there is one.
    Http {
        status: u16,
        retry_after: Option<SignedDuration>,
        body: String,
    },
    /// No reply: network, proxy, TLS or timeout. `reason` is a short fixed description, never
    /// the HTTP client's own message.
    Unreachable { reason: String },
}

/// The real environment of the machine mccboard runs on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply {
        crate::usage::call_endpoint(token)
    }
}
