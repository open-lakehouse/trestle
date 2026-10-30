//! Resolving the registered credential that governs a location.

use std::fmt;

use async_trait::async_trait;
use olai_storage_credentials::location::longest_match;
use olai_storage_credentials::{StorageLocation, StorageScope};

use crate::error::{Error, Result};
use crate::registered::RegisteredCredential;

/// Maps a storage location onto the registered credential allowed to mint for it.
///
/// A catalog resolves this through its external locations and the credential each one
/// references; [`ExternalLocations`] is that rule over an in-memory list. Hosts with
/// their own store of locations implement it themselves.
#[async_trait]
pub trait CredentialResolver: Send + Sync {
    /// The credential governing `location`, or [`Error::NotFound`].
    async fn resolve(&self, location: &StorageLocation) -> Result<RegisteredCredential>;
}

/// Registered locations, each with the credential that governs it.
///
/// A location is governed by the most specific registered location covering it,
/// matched on whole path segments: `s3://b/tab` governs `s3://b/tab/x`, never
/// `s3://b/table2`. Every spelling of a URL resolves the same way, since matching
/// compares canonical locations.
#[derive(Clone, Default)]
pub struct ExternalLocations {
    locations: Vec<(StorageScope, RegisteredCredential)>,
}

impl ExternalLocations {
    /// No locations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `scope`, governed by `credential`, replacing an existing registration
    /// of the same scope.
    pub fn with_location(mut self, scope: StorageScope, credential: RegisteredCredential) -> Self {
        self.locations.retain(|(existing, _)| *existing != scope);
        self.locations.push((scope, credential));
        self
    }
}

impl fmt::Debug for ExternalLocations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(
                self.locations
                    .iter()
                    .map(|(scope, credential)| (scope.to_string(), credential.kind())),
            )
            .finish()
    }
}

#[async_trait]
impl CredentialResolver for ExternalLocations {
    async fn resolve(&self, location: &StorageLocation) -> Result<RegisteredCredential> {
        longest_match(
            self.locations
                .iter()
                .map(|(scope, credential)| (scope, credential)),
            location,
        )
        .cloned()
        .ok_or_else(|| Error::NotFound {
            location: location.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registered::{AwsIamRole, AzureStorageKey};

    fn role(arn: &str) -> RegisteredCredential {
        RegisteredCredential::AwsIamRole(AwsIamRole {
            role_arn: arn.into(),
            ..AwsIamRole::default()
        })
    }

    fn arn(credential: &RegisteredCredential) -> &str {
        match credential {
            RegisteredCredential::AwsIamRole(r) => &r.role_arn,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn locations() -> ExternalLocations {
        ExternalLocations::new()
            .with_location("s3://bucket".parse().unwrap(), role("bucket"))
            .with_location("s3://bucket/tab".parse().unwrap(), role("tab"))
            .with_location(
                "abfss://ctr@acct.dfs.core.windows.net/data"
                    .parse()
                    .unwrap(),
                RegisteredCredential::AzureStorageKey(AzureStorageKey {
                    account_name: "acct".into(),
                    account_key: "a2V5".into(),
                }),
            )
    }

    async fn resolve(url: &str) -> Result<RegisteredCredential> {
        locations().resolve(&url.parse().unwrap()).await
    }

    #[tokio::test]
    async fn the_most_specific_location_governs() {
        assert_eq!(
            arn(&resolve("s3://bucket/tab/part-0").await.unwrap()),
            "tab"
        );
        // A sibling sharing the string prefix falls back to the bucket.
        assert_eq!(
            arn(&resolve("s3://bucket/table2/x").await.unwrap()),
            "bucket"
        );
        // Other spellings of the same location resolve the same way.
        assert_eq!(
            arn(&resolve("https://bucket.s3.us-east-1.amazonaws.com/tab/x")
                .await
                .unwrap()),
            "tab"
        );
        assert_eq!(
            resolve("https://acct.blob.core.windows.net/ctr/data/x")
                .await
                .unwrap()
                .kind(),
            "azure_storage_key"
        );
    }

    #[tokio::test]
    async fn unregistered_locations_are_not_found() {
        for url in [
            "s3://other/tab",
            "abfss://ctr@acct.dfs.core.windows.net/elsewhere",
        ] {
            assert!(
                matches!(resolve(url).await, Err(Error::NotFound { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn debug_shows_kinds_not_secrets() {
        let debug = format!("{:?}", locations());
        assert!(debug.contains("azure_storage_key"), "{debug}");
        assert!(!debug.contains("a2V5"), "{debug}");
    }
}
