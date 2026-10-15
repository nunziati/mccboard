//! Test support: the fake environment port. Enabled by the `testing` feature.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::{SignedDuration, Timestamp};

use crate::{AccessToken, EndpointReply, Environment};

/// A scriptable [`Environment`] for tests. Clones share state, so a test can keep a handle
/// after giving one to [`crate::Core::new`] and keep steering it.
#[derive(Debug, Clone)]
pub struct FakeEnvironment {
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Default)]
struct State {
    now: Timestamp,
    usage_replies: VecDeque<EndpointReply>,
    /// One entry per usage endpoint call: a fingerprint of its token, never the token.
    usage_calls: Vec<u64>,
}

impl FakeEnvironment {
    /// A fake whose clock reads `now` until moved.
    pub fn new(now: Timestamp) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                now,
                ..State::default()
            })),
        }
    }

    /// Sets the clock.
    pub fn set_now(&self, now: Timestamp) {
        self.state().now = now;
    }

    /// Moves the clock forward (or backward, with a negative duration).
    pub fn advance(&self, by: SignedDuration) {
        let mut state = self.state();
        state.now += by;
    }

    /// Queues the reply to the next usage endpoint call. With nothing queued, a call reads as
    /// unreachable.
    pub fn push_usage_reply(&self, reply: EndpointReply) {
        self.state().usage_replies.push_back(reply);
    }

    /// How many times the usage endpoint was called.
    pub fn usage_calls(&self) -> usize {
        self.state().usage_calls.len()
    }

    /// How many usage endpoint calls used the token `token`. The fake keeps fingerprints
    /// only, so the token never leaves its opaque type.
    pub fn usage_calls_with_token(&self, token: &str) -> usize {
        let fingerprint = crate::usage::fingerprint(token);
        self.state()
            .usage_calls
            .iter()
            .filter(|call| **call == fingerprint)
            .count()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Environment for FakeEnvironment {
    fn now(&self) -> Timestamp {
        self.state().now
    }

    fn call_usage_endpoint(&self, token: &AccessToken) -> EndpointReply {
        let mut state = self.state();
        state.usage_calls.push(token.fingerprint());
        state
            .usage_replies
            .pop_front()
            .unwrap_or(EndpointReply::Unreachable {
                reason: "no scripted reply".to_owned(),
            })
    }
}
