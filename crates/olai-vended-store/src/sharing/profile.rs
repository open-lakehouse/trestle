//! Sharing profiles: the file a provider gives a recipient.

use std::fmt;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Error, Result};

/// A sharing profile: the server's endpoint and the recipient's bearer token.
///
/// ```json
/// {
///   "shareCredentialsVersion": 1,
///   "endpoint": "https://sharing.example.com/api/1.0/opensharing",
///   "bearerToken": "<token>",
///   "expirationTime": "2030-01-01T00:00:00Z"
/// }
/// ```
///
/// `Debug` redacts the token.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SharingProfile {
    /// The profile format version.
    pub share_credentials_version: u32,
    /// The URL of the sharing server; request paths are relative to it.
    pub endpoint: String,
    /// The recipient's bearer token. Secret.
    pub bearer_token: String,
    /// When the token expires; `None` if it does not.
    #[serde(default)]
    pub expiration_time: Option<DateTime<Utc>>,
}

impl SharingProfile {
    /// Reads the profile file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|e| {
            Error::Config(format!(
                "cannot read sharing profile `{}`: {e}",
                path.display()
            ))
        })?;
        Self::from_json(&json)
    }

    /// Parses a profile from its JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        // serde_json errors can quote the input, which holds the token.
        serde_json::from_str(json).map_err(|e| {
            Error::Config(format!(
                "invalid sharing profile (line {}, column {})",
                e.line(),
                e.column()
            ))
        })
    }

    /// Whether the token has expired at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expiration_time.is_some_and(|expiry| expiry <= now)
    }
}

impl fmt::Debug for SharingProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharingProfile")
            .field("share_credentials_version", &self.share_credentials_version)
            .field("endpoint", &self.endpoint)
            .field("bearer_token", &"<redacted>")
            .field("expiration_time", &self.expiration_time)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"{
        "shareCredentialsVersion": 1,
        "endpoint": "https://sharing.example.com/api/1.0/opensharing",
        "bearerToken": "s3cr3t",
        "expirationTime": "2030-01-01T00:00:00Z"
    }"#;

    #[test]
    fn parses_and_redacts() {
        let profile = SharingProfile::from_json(PROFILE).unwrap();
        assert_eq!(
            profile.endpoint,
            "https://sharing.example.com/api/1.0/opensharing"
        );
        assert!(!profile.is_expired("2029-12-31T00:00:00Z".parse().unwrap()));
        assert!(profile.is_expired("2030-01-01T00:00:00Z".parse().unwrap()));
        assert!(!format!("{profile:?}").contains("s3cr3t"));
    }

    #[test]
    fn parse_errors_do_not_quote_the_token() {
        let err =
            SharingProfile::from_json(r#"{"bearerToken": "s3cr3t", "endpoint": 1}"#).unwrap_err();
        assert!(!err.to_string().contains("s3cr3t"), "{err}");
    }
}
