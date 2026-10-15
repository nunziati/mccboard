use serde::{Deserialize, Serialize};

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
