//! A minimal Open Sharing client: asset lookups and credential vending.

use std::fmt;

use olai_http::CloudClient;
use olai_storage_credentials::wire::sharing::SharingTemporaryCredentials;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api::{ApiClient, Dialect};
use crate::error::Result;

/// Sharing errors: `{"errorCode": "...", "message": "..."}`, as Delta Sharing writes
/// them.
const DIALECT: Dialect = Dialect {
    service: "Open Sharing",
    envelope: |body| {
        #[derive(Deserialize)]
        struct Envelope {
            #[serde(alias = "error_code")]
            #[serde(rename = "errorCode")]
            error_code: Option<String>,
            message: Option<String>,
        }
        serde_json::from_str::<Envelope>(body)
            .ok()
            .map(|e| (e.error_code, e.message))
    },
};

/// A kind of storage-backed asset Open Sharing vends credentials for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AssetKind {
    /// A volume: files under a storage location.
    Volume,
    /// An agent skill: a packaged directory under a storage location.
    Skill,
}

impl AssetKind {
    /// The path segment of the asset collection, and the asset's credentials endpoint.
    fn segments(self) -> (&'static str, &'static str) {
        match self {
            Self::Volume => ("volumes", "temporary-volume-credentials"),
            Self::Skill => ("skills", "temporary-skill-credentials"),
        }
    }
}

/// A client for the Open Sharing endpoints vending needs: `GET` a volume or skill for
/// its storage location, and `POST` its `temporary-*-credentials`.
///
/// Authentication, retries and the I/O runtime belong to the wrapped [`CloudClient`].
/// Cheap to clone.
#[derive(Clone)]
pub struct SharingClient {
    api: ApiClient,
}

impl SharingClient {
    /// A client for the server at `endpoint`, the profile's `endpoint` (for example
    /// `https://sharing.example.com/api/1.0/opensharing`).
    pub fn new(client: CloudClient, endpoint: Url) -> Self {
        Self {
            api: ApiClient::new(client, endpoint, DIALECT),
        }
    }

    /// The server endpoint.
    pub fn endpoint(&self) -> &Url {
        self.api.base_url()
    }

    /// Looks up the asset `share.schema.name`.
    pub async fn get_asset(
        &self,
        kind: AssetKind,
        share: &str,
        schema: &str,
        name: &str,
    ) -> Result<AssetInfo> {
        let (collection, _) = kind.segments();
        let request = self
            .api
            .get(&["shares", share, "schemas", schema, collection, name])?;
        self.api.send(request).await
    }

    /// Vends read credentials for the asset `share.schema.name`.
    pub async fn temporary_credentials(
        &self,
        kind: AssetKind,
        share: &str,
        schema: &str,
        name: &str,
    ) -> Result<SharingTemporaryCredentials> {
        // The asset is named by the path; the body carries nothing else.
        #[derive(Serialize)]
        struct Empty {}
        let (collection, endpoint) = kind.segments();
        let request = self
            .api
            .post(&[
                "shares", share, "schemas", schema, collection, name, endpoint,
            ])?
            .json(&Empty {});
        self.api.send(request).await
    }
}

impl fmt::Debug for SharingClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharingClient")
            .field("endpoint", &self.endpoint().as_str())
            .finish_non_exhaustive()
    }
}

/// The fields of a `SharingVolume` or `SharingSkill` vending needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    /// The asset name.
    pub name: String,
    /// The asset's storage root.
    #[serde(default, alias = "storage_location")]
    pub storage_location: Option<String>,
}
