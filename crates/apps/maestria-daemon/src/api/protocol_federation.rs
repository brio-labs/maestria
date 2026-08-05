use std::fmt;

use anyhow::{Result, anyhow};
use maestria_domain::RealmId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{EvidenceResponse, SearchResponse};

/// Raw federation bearer credential. Its value is intentionally redacted from
/// debug output and only crosses the owner-only local socket or binding file.
#[derive(Clone, PartialEq, Eq)]
pub struct FederationCredential(String);

impl FederationCredential {
    pub(crate) fn try_from(value: String) -> Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(anyhow!(
                "federation credential must be 64 lowercase hexadecimal characters"
            ));
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FederationCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FederationCredential([REDACTED])")
    }
}

impl Serialize for FederationCredential {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FederationCredential {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientAuthentication {
    InstanceToken {
        token: String,
    },
    FederationGrant {
        consumer_realm: RealmId,
        credential: FederationCredential,
    },
}

impl fmt::Debug for ClientAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstanceToken { .. } => formatter
                .debug_struct("InstanceToken")
                .field("token", &"[REDACTED]")
                .finish(),
            Self::FederationGrant {
                consumer_realm,
                credential,
            } => formatter
                .debug_struct("FederationGrant")
                .field("consumer_realm", consumer_realm)
                .field("credential", credential)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RealmGrantAccess {
    SearchOnly,
    SearchAndOpenEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RealmGrantSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmGrantResponse {
    pub token_digest: String,
    pub provider_realm: RealmId,
    pub consumer_realm: RealmId,
    pub access: RealmGrantAccess,
    pub max_sensitivity: RealmGrantSensitivity,
    pub max_results: usize,
    pub max_evidence_bytes: usize,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmGrantCreatedResponse {
    pub grant: RealmGrantResponse,
    pub credential: FederationCredential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmGrantListResponse {
    pub grants: Vec<RealmGrantResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationSearchResponse {
    pub provider_realm: RealmId,
    pub graph_degraded: bool,
    pub search: SearchResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationEvidenceResponse {
    pub provider_realm: RealmId,
    pub evidence: EvidenceResponse,
}
