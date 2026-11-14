//! A minimal Neon control-plane client (feature `neon`): branch storage discovery and
//! branch credentials.
//!
//! Neon branches carry S3-compatible object storage. Its credentials are minted by the
//! control plane (`https://console.neon.tech/api/v2`, authenticated with a Neon API
//! key): branch-scoped keys with `storage:read` and/or `storage:write`, optionally with
//! an enforced expiry. A key is valid on its branch and on the branch's descendants.
//!
//! ```no_run
//! # async fn run() -> Result<(), olai_http::neon::Error> {
//! use olai_http::CloudClient;
//! use olai_http::neon::{CreateCredential, DEFAULT_API_URL, NeonClient};
//!
//! let neon = NeonClient::new(
//!     CloudClient::new_with_token("<api key>"),
//!     DEFAULT_API_URL.parse().unwrap(),
//! );
//! let storage = neon.branch_storage("<project>", "br-winter-pond-aptw82ef").await?;
//! let created = neon
//!     .create_credential_with(
//!         "<project>",
//!         "br-winter-pond-aptw82ef",
//!         &CreateCredential::new(["storage:read"]).with_name("reader"),
//!     )
//!     .await?;
//! # let _ = (storage, created);
//! # Ok(()) }
//! ```

use std::error::Error as StdError;
use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{CloudClient, CloudRequestBuilder, RetryConfig, RetryError};

/// The Neon API.
pub const DEFAULT_API_URL: &str = "https://console.neon.tech/api/v2/";

/// The scope that lets a credential read and list a branch's buckets.
pub const SCOPE_STORAGE_READ: &str = "storage:read";

/// The scope that lets a credential write and delete in a branch's buckets.
pub const SCOPE_STORAGE_WRITE: &str = "storage:write";

/// Characters escaped in one URL path segment, so a name can never introduce a new
/// segment. `\` separates segments in http(s) URLs too.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// A `Result` defaulting to this module's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors of the Neon control-plane client.
///
/// Messages never include secret material.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// The control plane refused the request or could not be reached.
    #[error(transparent)]
    Api(#[from] ApiError),

    /// A project, branch or credential id cannot be used as a URL path segment.
    #[error("invalid Neon reference: {0}")]
    InvalidReference(String),

    /// The control plane answered with a body this client cannot read.
    #[error("Neon returned an invalid response body: {0}")]
    InvalidResponse(String),
}

/// A request the Neon control plane refused or could not answer.
///
/// Carries the HTTP status and Neon's error code when the server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status, if the server answered.
    pub status: Option<u16>,
    /// Neon's error code, if any (often empty).
    pub code: Option<String>,
    /// What went wrong, with Neon's `reason` (`org_not_entitled`, ...) appended.
    pub message: String,
}

impl ApiError {
    fn from_response(status: Option<u16>, body: &str) -> Self {
        #[derive(Deserialize)]
        struct Envelope {
            code: Option<String>,
            message: Option<String>,
            /// Why branch storage is unavailable (`org_not_entitled`, ...).
            reason: Option<String>,
        }
        let (code, message) = match serde_json::from_str::<Envelope>(body) {
            Ok(e) => {
                let message = match (e.message, e.reason) {
                    (Some(message), Some(reason)) => Some(format!("{message} ({reason})")),
                    (message, reason) => message.or(reason),
                };
                (e.code, message)
            }
            Err(_) => (None, Some(body.chars().take(512).collect())),
        };
        Self {
            status,
            code,
            message: message.unwrap_or_default(),
        }
    }

    fn from_transport(err: &crate::Error) -> Self {
        let mut source: Option<&(dyn StdError + 'static)> = Some(err);
        while let Some(current) = source {
            if let Some(retry) = current.downcast_ref::<RetryError>() {
                return Self::from_response(
                    retry.status().map(|s| s.as_u16()),
                    retry.body().unwrap_or_default(),
                );
            }
            source = current.source();
        }
        Self {
            status: None,
            code: None,
            message: err.to_string(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Neon request failed")?;
        match (self.status, self.code.as_deref().filter(|c| !c.is_empty())) {
            (Some(status), Some(code)) => write!(f, " ({status} {code})")?,
            (Some(status), None) => write!(f, " ({status})")?,
            (None, Some(code)) => write!(f, " ({code})")?,
            (None, None) => {}
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

impl StdError for ApiError {}

/// A client for the Neon endpoints object storage needs: a branch's storage, and
/// creating, revealing, listing and revoking its credentials.
///
/// Authentication (a Neon API key), retries and the I/O runtime belong to the wrapped
/// [`CloudClient`]. Creating a credential is never retried: a retry after a lost
/// response would create another credential nobody holds. Cheap to clone.
#[derive(Clone)]
pub struct NeonClient {
    client: CloudClient,
    /// For requests that must not be retried.
    once: CloudClient,
    base_url: Url,
}

impl NeonClient {
    /// A client for the Neon API at `api_url` (see [`DEFAULT_API_URL`]); a trailing `/`
    /// is added if missing.
    pub fn new(client: CloudClient, api_url: Url) -> Self {
        let once = client.clone().with_retry_config(RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        });
        Self {
            client,
            once,
            base_url: with_trailing_slash(api_url),
        }
    }

    /// The API base URL.
    pub fn api_url(&self) -> &Url {
        &self.base_url
    }

    /// The object storage of a branch. A 404 means storage is not available for it.
    pub async fn branch_storage(&self, project: &str, branch: &str) -> Result<BranchStorage> {
        let url = self.url(&["projects", project, "branches", branch, "storage"])?;
        send(self.client.get(url)).await
    }

    /// The secret of the branch credential `token_id`.
    ///
    /// Answers 409 for credentials issued before secrets could be revealed (those must
    /// be rotated first), and 404 for expired ones.
    pub async fn reveal_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<CredentialSecret> {
        let url = self.url(&[
            "projects",
            project,
            "branches",
            branch,
            "credentials",
            token_id,
            "reveal",
        ])?;
        send(self.client.post(url)).await
    }

    /// Creates a branch credential with `scopes` ([`SCOPE_STORAGE_READ`],
    /// [`SCOPE_STORAGE_WRITE`]) and an optional `name`, that never expires.
    ///
    /// Sent once, without retries. See [`create_credential_with`](Self::create_credential_with)
    /// to set an expiry.
    pub async fn create_credential(
        &self,
        project: &str,
        branch: &str,
        scopes: &[&str],
        name: Option<&str>,
    ) -> Result<CredentialSecret> {
        let mut request = CreateCredential::new(scopes.iter().copied());
        request.name = name.map(str::to_owned);
        let created = self
            .create_credential_with(project, branch, &request)
            .await?;
        Ok(created.secret)
    }

    /// Creates a branch credential as `request` describes.
    ///
    /// Sent once, without retries: a retried create would leave a credential nobody
    /// holds. Neon refuses an `expires_at` outside 1 minute to 365 days from now (400).
    pub async fn create_credential_with(
        &self,
        project: &str,
        branch: &str,
        request: &CreateCredential,
    ) -> Result<CreatedCredential> {
        #[derive(Serialize)]
        struct Body<'a> {
            scopes: &'a [String],
            principal_type: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            expires_at: Option<String>,
        }
        let url = self.url(&["projects", project, "branches", branch, "credentials"])?;
        let body = Body {
            scopes: &request.scopes,
            principal_type: "user",
            name: request.name.as_deref(),
            expires_at: request
                .expires_at
                .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
        };
        send(self.once.post(url).json(&body)).await
    }

    /// The credentials visible on a branch, without secrets.
    ///
    /// Neon lists a branch's own credentials and those of its ancestors (which are
    /// valid on it too), and keeps expired and revoked ones in the list: filter on
    /// [`BranchCredential::branch_id`], [`expires_at`](BranchCredential::expires_at)
    /// and [`revoked_at`](BranchCredential::revoked_at).
    pub async fn list_credentials(
        &self,
        project: &str,
        branch: &str,
    ) -> Result<Vec<BranchCredential>> {
        #[derive(Deserialize)]
        struct Response {
            #[serde(default)]
            credentials: Vec<BranchCredential>,
        }
        let url = self.url(&["projects", project, "branches", branch, "credentials"])?;
        let response: Response = send(self.client.get(url)).await?;
        Ok(response.credentials)
    }

    /// Revokes the branch credential `token_id`. Revoking a credential twice succeeds;
    /// an unknown one answers 404.
    ///
    /// Revocation is eventually consistent: S3 may accept the key for some seconds
    /// after this returns. Rely on expiry, not revocation, to bound a key's life.
    pub async fn revoke_credential(
        &self,
        project: &str,
        branch: &str,
        token_id: &str,
    ) -> Result<()> {
        let url = self.url(&[
            "projects",
            project,
            "branches",
            branch,
            "credentials",
            token_id,
        ])?;
        let response = self
            .client
            .delete(url)
            .send()
            .await
            .map_err(|err| ApiError::from_transport(&err))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::from_response(Some(status.as_u16()), &body).into());
        }
        Ok(())
    }

    /// The URL of `segments` below the base, each escaped as one path segment.
    ///
    /// A `.` or `..` segment is refused: URL resolution would apply it and climb out of
    /// the base.
    fn url(&self, segments: &[&str]) -> Result<Url> {
        if let Some(dots) = segments.iter().find(|s| is_dot_segment(s)) {
            return Err(Error::InvalidReference(format!(
                "`{dots}` is not a valid name"
            )));
        }
        let path = segments
            .iter()
            .map(|s| utf8_percent_encode(s, PATH_SEGMENT).to_string())
            .collect::<Vec<_>>()
            .join("/");
        let url = self
            .base_url
            .join(&path)
            .map_err(|e| Error::InvalidReference(e.to_string()))?;
        if !url.as_str().starts_with(self.base_url.as_str()) {
            return Err(Error::InvalidReference(format!(
                "`{}` leaves the API base",
                segments.join("/")
            )));
        }
        Ok(url)
    }
}

impl fmt::Debug for NeonClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeonClient")
            .field("api_url", &self.base_url.as_str())
            .finish_non_exhaustive()
    }
}

/// Sends `request` and decodes a success body, or turns the failure into an
/// [`ApiError`].
///
/// olai-http reports non-success statuses as errors carrying a [`RetryError`] (and,
/// with its `recording` feature, as responses), so both are handled.
async fn send<T: DeserializeOwned>(request: CloudRequestBuilder) -> Result<T> {
    let response = request
        .send()
        .await
        .map_err(|err| ApiError::from_transport(&err))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError::from_response(Some(status.as_u16()), &body).into());
    }
    response
        .json()
        .await
        .map_err(|e| Error::InvalidResponse(e.to_string()))
}

/// Whether `segment` is `.` or `..`, spelled plainly or percent-encoded in any case.
fn is_dot_segment(segment: &str) -> bool {
    let decoded = percent_decode_str(segment).decode_utf8_lossy();
    matches!(decoded.as_ref(), "." | "..")
}

fn with_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url
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

/// What to create: see [`NeonClient::create_credential_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateCredential {
    /// The scopes: [`SCOPE_STORAGE_READ`], [`SCOPE_STORAGE_WRITE`].
    pub scopes: Vec<String>,
    /// A label for the credential.
    pub name: Option<String>,
    /// When Neon stops accepting it; never when `None`. Sent with second precision.
    pub expires_at: Option<DateTime<Utc>>,
}

impl CreateCredential {
    /// A credential with `scopes`, no name and no expiry.
    pub fn new<S: Into<String>>(scopes: impl IntoIterator<Item = S>) -> Self {
        Self {
            scopes: scopes.into_iter().map(Into::into).collect(),
            name: None,
            expires_at: None,
        }
    }

    /// Sets the label.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the expiry.
    pub fn with_expires_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

/// A created credential (`CreateCredentialResponse`): its secret and metadata.
///
/// `Debug` redacts the secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CreatedCredential {
    /// The credential id and S3 secret.
    #[serde(flatten)]
    pub secret: CredentialSecret,
    /// The label, if any.
    #[serde(default)]
    pub name: Option<String>,
    /// The granted scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// When it expires; never when `None`.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for CreatedCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedCredential")
            .field("secret", &self.secret)
            .field("name", &self.name)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A credential as listed (`Credential`): metadata only, no secret.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct BranchCredential {
    /// The credential id, which is the S3 access key id.
    pub token_id: String,
    /// The label, if any.
    #[serde(default)]
    pub name: Option<String>,
    /// The granted scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The branch it was created on: the listed branch or one of its ancestors.
    pub branch_id: String,
    /// When it was created.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// When it expires; never when `None`.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    /// When it was revoked, if it was.
    #[serde(default)]
    pub revoked_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server};
    use serde_json::json;

    use super::*;

    const BRANCH: &str = "/projects/proj/branches/br-1";

    fn client(server: &Server) -> NeonClient {
        NeonClient::new(
            CloudClient::new_with_token("napi-key"),
            server.url().parse().unwrap(),
        )
    }

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn creates_expiring_named_credentials_once() {
        let mut server = Server::new_async().await;
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .match_header("authorization", "Bearer napi-key")
            .match_body(Matcher::Json(json!({
                "scopes": ["storage:read", "storage:write"],
                "principal_type": "user",
                "name": "olai-vend-read_write-1",
                "expires_at": "2026-11-14T01:00:00Z",
            })))
            .with_status(201)
            .with_body(
                json!({
                    "token_id": "nak_live_new",
                    "token_id_short": "new",
                    "name": "olai-vend-read_write-1",
                    "api_token": "napi_token",
                    "s3_secret_access_key": "nsk_live_new",
                    "scopes": ["storage:read", "storage:write"],
                    "branch_id": "br-1",
                    "created_at": "2026-11-14T00:00:00Z",
                    "expires_at": "2026-11-14T01:00:00Z",
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let created = client(&server)
            .create_credential_with(
                "proj",
                "br-1",
                &CreateCredential::new([SCOPE_STORAGE_READ, SCOPE_STORAGE_WRITE])
                    .with_name("olai-vend-read_write-1")
                    // Sub-second precision is dropped on the wire.
                    .with_expires_at(t("2026-11-14T01:00:00.250Z")),
            )
            .await
            .unwrap();
        assert_eq!(created.secret.token_id, "nak_live_new");
        assert_eq!(created.secret.s3_secret_access_key, "nsk_live_new");
        assert_eq!(created.expires_at, Some(t("2026-11-14T01:00:00Z")));
        assert_eq!(created.scopes, ["storage:read", "storage:write"]);
        let debug = format!("{created:?}");
        assert!(!debug.contains("nsk_live_new"), "{debug}");
        create.assert_async().await;
    }

    #[tokio::test]
    async fn creates_are_not_retried() {
        let mut server = Server::new_async().await;
        let create = server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .with_status(502)
            .expect(1)
            .create_async()
            .await;
        let err = client(&server)
            .create_credential("proj", "br-1", &[SCOPE_STORAGE_READ], None)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Api(api) if api.status == Some(502)),
            "{err}"
        );
        create.assert_async().await;
    }

    #[tokio::test]
    async fn bad_expiries_report_neons_message() {
        let mut server = Server::new_async().await;
        server
            .mock("POST", format!("{BRANCH}/credentials").as_str())
            .with_status(400)
            .with_body(
                json!({
                    "code": "",
                    "message": "expires_at must be between 1 minute and 365 days from now",
                })
                .to_string(),
            )
            .create_async()
            .await;
        let err = client(&server)
            .create_credential_with(
                "proj",
                "br-1",
                &CreateCredential::new([SCOPE_STORAGE_READ])
                    .with_expires_at(t("2000-01-01T00:00:00Z")),
            )
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Neon request failed (400): expires_at must be between 1 minute and 365 days from now"
        );
    }

    #[tokio::test]
    async fn lists_credentials_with_lineage_expiry_and_revocation() {
        let mut server = Server::new_async().await;
        // The shape Neon answers with: ancestors' credentials included, expired and
        // revoked ones kept, absent fields omitted.
        server
            .mock("GET", format!("{BRANCH}/credentials").as_str())
            .with_body(
                json!({ "credentials": [
                    {
                        "token_id": "nak_live_parent", "token_id_short": "parent",
                        "name": "Default object storage credential",
                        "scopes": ["storage:read", "storage:write"],
                        "branch_id": "br-0", "principal_type": "user",
                        "created_at": "2026-10-27T20:10:54Z",
                    },
                    {
                        "token_id": "nak_live_old", "token_id_short": "old",
                        "name": "olai-vend-read-1", "scopes": ["storage:read"],
                        "branch_id": "br-1", "principal_type": "user",
                        "created_at": "2026-11-13T23:39:44Z",
                        "expires_at": "2026-11-13T23:41:43Z",
                        "revoked_at": "2026-11-13T23:43:18Z",
                    },
                ]})
                .to_string(),
            )
            .create_async()
            .await;
        let listed = client(&server)
            .list_credentials("proj", "br-1")
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].branch_id, "br-0");
        assert_eq!(listed[0].expires_at, None);
        assert_eq!(listed[0].revoked_at, None);
        assert_eq!(listed[1].name.as_deref(), Some("olai-vend-read-1"));
        assert_eq!(listed[1].expires_at, Some(t("2026-11-13T23:41:43Z")));
        assert_eq!(listed[1].revoked_at, Some(t("2026-11-13T23:43:18Z")));
    }

    #[tokio::test]
    async fn revokes_credentials() {
        let mut server = Server::new_async().await;
        let revoke = server
            .mock(
                "DELETE",
                format!("{BRANCH}/credentials/nak_live_1").as_str(),
            )
            .with_status(204)
            .expect(1)
            .create_async()
            .await;
        server
            .mock(
                "DELETE",
                format!("{BRANCH}/credentials/nak_live_gone").as_str(),
            )
            .with_status(404)
            .with_body(
                json!({ "code": "", "message": "credential or project not found" }).to_string(),
            )
            .create_async()
            .await;
        let neon = client(&server);
        neon.revoke_credential("proj", "br-1", "nak_live_1")
            .await
            .unwrap();
        revoke.assert_async().await;
        let err = neon
            .revoke_credential("proj", "br-1", "nak_live_gone")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Api(api) if api.status == Some(404)),
            "{err}"
        );
    }

    #[tokio::test]
    async fn storage_errors_carry_the_reason() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", format!("{BRANCH}/storage").as_str())
            .with_status(404)
            .with_body(
                json!({
                    "code": "",
                    "message": "object storage is not enabled",
                    "reason": "org_not_entitled",
                })
                .to_string(),
            )
            .create_async()
            .await;
        let err = client(&server)
            .branch_storage("proj", "br-1")
            .await
            .unwrap_err();
        let Error::Api(api) = &err else {
            panic!("expected an API error, got {err}");
        };
        assert_eq!(api.status, Some(404));
        assert!(api.message.contains("org_not_entitled"), "{err}");
    }

    #[test]
    fn ids_stay_one_segment() {
        let neon = NeonClient::new(
            CloudClient::new_with_token("SECRET-napi"),
            "https://console.neon.tech/api/v2".parse().unwrap(),
        );
        assert_eq!(
            neon.url(&["projects", "a/b?c", "branches", r"x\..\y"])
                .unwrap()
                .as_str(),
            "https://console.neon.tech/api/v2/projects/a%2Fb%3Fc/branches/x%5C..%5Cy"
        );
        for dots in [".", "..", "%2e%2E"] {
            assert!(matches!(
                neon.url(&["projects", dots]),
                Err(Error::InvalidReference(_))
            ));
        }
        assert!(!format!("{neon:?}").contains("SECRET"));
    }
}
