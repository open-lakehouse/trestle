//! A minimal Neon control-plane client: branch storage discovery and branch
//! credentials.

use std::fmt;

use olai_http::{CloudClient, RetryConfig};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api::{ApiClient, Dialect};
use crate::error::Result;

/// The Neon API.
pub const DEFAULT_API_URL: &str = "https://console.neon.tech/api/v2/";

/// Neon errors: `{"code": "...", "message": "..."}`.
const DIALECT: Dialect = Dialect {
    service: "Neon",
    envelope: |body| {
        #[derive(Deserialize)]
        struct Envelope {
            code: Option<String>,
            message: Option<String>,
            /// Why branch storage is unavailable (`org_not_entitled`, ...).
            reason: Option<String>,
        }
        serde_json::from_str::<Envelope>(body).ok().map(|e| {
            let message = match (e.message, e.reason) {
                (Some(message), Some(reason)) => Some(format!("{message} ({reason})")),
                (message, reason) => message.or(reason),
            };
            (e.code, message)
        })
    },
};

/// A client for the Neon endpoints object storage needs: `GET .../storage`, and
/// `POST .../credentials` to create or reveal a branch credential.
///
/// Authentication (a Neon API key), retries and the I/O runtime belong to the wrapped
/// [`CloudClient`]. Creating a credential is never retried: a retry after a lost
/// response would create another long-lived credential. Cheap to clone.
#[derive(Clone)]
pub struct NeonClient {
    api: ApiClient,
    /// For requests that must not be retried.
    once: ApiClient,
}

impl NeonClient {
    /// A client for the Neon API at `api_url` (see [`DEFAULT_API_URL`]).
    pub fn new(client: CloudClient, api_url: Url) -> Self {
        let once = client.clone().with_retry_config(RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        });
        Self {
            api: ApiClient::new(client, api_url.clone(), DIALECT),
            once: ApiClient::new(once, api_url, DIALECT),
        }
    }

    /// The API base URL.
    pub fn api_url(&self) -> &Url {
        self.api.base_url()
    }

    /// The object storage of a branch. A 404 means storage is not available for it.
    pub async fn branch_storage(&self, project: &str, branch: &str) -> Result<BranchStorage> {
        let request = self
            .api
            .get(&["projects", project, "branches", branch, "storage"])?;
        self.api.send(request).await
    }

    /// The secret of the branch credential `token_id`.
    ///
    /// Answers 409 for credentials issued before secrets could be revealed; those must
    /// be rotated first.
    pub async fn reveal_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<CredentialSecret> {
        let request = self.api.post(&[
            "projects",
            project,
            "branches",
            branch,
            "credentials",
            token_id,
            "reveal",
        ])?;
        self.api.send(request).await
    }

    /// Creates a branch credential with `scopes` (`storage:read`, `storage:write`).
    ///
    /// Sent once, without retries: Neon credentials do not expire, so a retried create
    /// would leave a credential nobody holds.
    pub async fn create_credential(
        &self,
        project: &str,
        branch: &str,
        scopes: &[&str],
        name: Option<&str>,
    ) -> Result<CredentialSecret> {
        #[derive(Serialize)]
        struct Request<'a> {
            scopes: &'a [&'a str],
            principal_type: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
        }
        let request = self
            .once
            .post(&["projects", project, "branches", branch, "credentials"])?
            .json(&Request {
                scopes,
                principal_type: "user",
                name,
            });
        self.once.send(request).await
    }
}

impl fmt::Debug for NeonClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonClient")
            .field("api_url", &self.api_url().as_str())
            .finish_non_exhaustive()
    }
}

/// A branch's object storage (`BranchStorage`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BranchStorage {
    /// Whether object storage is enabled.
    pub enabled: bool,
    /// The branch's S3 endpoint.
    pub s3_endpoint: String,
    /// Its region.
    pub region: String,
    /// Whether clients must use path-style addressing.
    pub force_path_style: bool,
}

/// The secret of a branch credential (`CredentialSecret`, or the
/// `CreateCredentialResponse` that includes it).
///
/// `Debug` redacts the secrets.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct CredentialSecret {
    /// The credential id, which is the S3 access key id.
    pub token_id: String,
    /// The S3 secret access key. Secret.
    pub s3_secret_access_key: String,
}

impl fmt::Debug for CredentialSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialSecret")
            .field("token_id", &self.token_id)
            .field("s3_secret_access_key", &"<redacted>")
            .finish()
    }
}
