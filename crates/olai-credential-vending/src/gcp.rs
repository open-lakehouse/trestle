//! Minting GCS tokens downscoped by a Credential Access Boundary.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use olai_http::bearer::GcpServiceAccount as KeySource;
use olai_http::gcp::{
    CLOUD_PLATFORM_SCOPE, GcpCredentialProvider, GcpToken, GcpTokenOptions, GoogleBuilder,
    downscope_token, impersonate_access_token, service_account_access_token,
};
use olai_http::{ClientOptions, RetryConfig};
use olai_storage_credentials::policy::gcs::access_boundary;
use olai_storage_credentials::{StorageCredentials, StorageProvider, StoreHints, VendedCredential};

use crate::error::{Error, Result};
use crate::mint::{CredentialMinter, MintRequest};
use crate::registered::{GcpServiceAccount, GcpServiceAccountKey};

/// The lifetime of a token when the request names none.
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// The longest token minted by default: Google's limit for impersonated tokens, unless
/// an organization policy extends it (see [`GcsDownscopedMinter::with_max_ttl`]).
const MAX_TTL: Duration = Duration::from_secs(3600);

/// The shortest token minted.
const MIN_TTL: Duration = Duration::from_secs(60);

/// Retries for each token call: a mint waits on them, so it fails within seconds, not
/// the default three minutes.
fn token_retry() -> RetryConfig {
    RetryConfig {
        max_retries: 3,
        retry_timeout: Duration::from_secs(30),
        ..RetryConfig::default()
    }
}

/// Where the token that is downscoped comes from.
enum Source {
    /// Another identity impersonates the account.
    Impersonate(GcpCredentialProvider),
    /// A key of the account.
    Key(KeySource),
}

/// Mints GCS tokens for a service account, downscoped to the requested scope.
///
/// Each mint gets an access token for the account, then exchanges it at Google's STS
/// with a Credential Access Boundary (P1's
/// [`access_boundary`](olai_storage_credentials::policy::gcs::access_boundary)) that
/// restricts the token to the requested bucket, prefix and access, whatever the
/// account itself may do. A downscoped token cannot outlive the token it was exchanged
/// from, so the source token is fetched fresh for each mint:
///
/// - impersonated (no key registered), it lives as long as the request asks, clamped
///   to between a minute and [`with_max_ttl`](Self::with_max_ttl) (one hour by
///   default);
/// - from a key, it always lives one hour: Google's token endpoint issues no other.
///
/// GCP has no session name: [`MintRequest::session_name`] is ignored, and Cloud Audit
/// Logs name the service account.
pub struct GcsDownscopedMinter {
    email: String,
    source: Source,
    iam_credentials_endpoint: Option<String>,
    token_endpoint: Option<String>,
    sts_endpoint: Option<String>,
    gcs_endpoint: Option<String>,
    client_options: ClientOptions,
    max_ttl: Duration,
}

impl GcsDownscopedMinter {
    /// A minter for `account`: with its key when one is registered, else impersonating
    /// it with the host's own GCP identity (application default credentials, the
    /// metadata server or workload identity federation).
    ///
    /// Fails if the key cannot be read, or belongs to another account.
    pub fn new(account: &GcpServiceAccount) -> Result<Self> {
        let source = match &account.key {
            None => Source::Impersonate(GoogleBuilder::from_env().build(None)?.credentials),
            Some(key) => {
                let key = match key {
                    GcpServiceAccountKey::ServiceAccountKey(json) => KeySource::Key(json.clone()),
                    GcpServiceAccountKey::ServiceAccountKeyFile(path) => {
                        KeySource::File(path.clone())
                    }
                };
                check_key_account(&key, &account.email)?;
                Source::Key(key)
            }
        };
        Ok(Self {
            email: account.email.clone(),
            source,
            iam_credentials_endpoint: None,
            token_endpoint: None,
            sts_endpoint: None,
            gcs_endpoint: None,
            client_options: ClientOptions::default(),
            max_ttl: MAX_TTL,
        })
    }

    /// Impersonates the account with `base` instead, whether or not a key is
    /// registered.
    pub fn with_base_credentials(mut self, base: GcpCredentialProvider) -> Self {
        self.source = Source::Impersonate(base);
        self
    }

    /// Calls the IAM Service Account Credentials API at `endpoint` instead of
    /// `https://iamcredentials.googleapis.com`.
    pub fn with_iam_credentials_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.iam_credentials_endpoint = Some(endpoint.into());
        self
    }

    /// Requests key tokens from `endpoint` instead of the key's `token_uri`.
    pub fn with_token_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.token_endpoint = Some(endpoint.into());
        self
    }

    /// Exchanges tokens at `endpoint` instead of `https://sts.googleapis.com/v1/token`.
    pub fn with_sts_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.sts_endpoint = Some(endpoint.into());
        self
    }

    /// The GCS-compatible endpoint the minted tokens are for, passed to clients as a
    /// store hint.
    pub fn with_gcs_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.gcs_endpoint = Some(endpoint.into());
        self
    }

    /// HTTP client options for the token calls.
    pub fn with_client_options(mut self, options: ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// The longest lifetime an impersonated token is requested for. Google allows up
    /// to twelve hours where the organization policy
    /// `iam.allowServiceAccountCredentialLifetimeExtension` admits the account.
    pub fn with_max_ttl(mut self, max_ttl: Duration) -> Self {
        self.max_ttl = max_ttl.max(MIN_TTL);
        self
    }

    fn options(&self, endpoint: &Option<String>) -> GcpTokenOptions {
        GcpTokenOptions {
            endpoint: endpoint.clone(),
            client_options: self.client_options.clone(),
            retry: token_retry(),
        }
    }

    /// A fresh token for the account, to downscope.
    async fn source_token(&self, ttl: Option<Duration>) -> Result<GcpToken> {
        Ok(match &self.source {
            Source::Impersonate(base) => {
                let lifetime = ttl.unwrap_or(DEFAULT_TTL).clamp(MIN_TTL, self.max_ttl);
                impersonate_access_token(
                    base,
                    &self.email,
                    lifetime,
                    &[CLOUD_PLATFORM_SCOPE],
                    &self.options(&self.iam_credentials_endpoint),
                )
                .await?
            }
            Source::Key(key) => {
                service_account_access_token(
                    key,
                    CLOUD_PLATFORM_SCOPE,
                    &self.options(&self.token_endpoint),
                )
                .await?
            }
        })
    }

    fn hints(&self) -> StoreHints {
        match &self.gcs_endpoint {
            Some(endpoint) => StoreHints {
                endpoint: Some(endpoint.clone()),
                allow_http: Some(endpoint.starts_with("http://")),
                ..StoreHints::default()
            },
            None => StoreHints::default(),
        }
    }
}

/// Fails unless `key` is a key of `email`.
fn check_key_account(key: &KeySource, email: &str) -> Result<()> {
    let json = match key {
        KeySource::Key(json) => json.clone(),
        KeySource::File(path) => std::fs::read_to_string(path).map_err(|e| {
            Error::unsupported(email, format!("cannot read the key file `{path}`: {e}"))
        })?,
    };
    #[derive(serde::Deserialize)]
    struct Account {
        client_email: Option<String>,
    }
    let account: Account = serde_json::from_str(&json)
        .map_err(|_| Error::unsupported(email, "the key is not a service-account JSON key"))?;
    match account.client_email {
        Some(client_email) if client_email == email => Ok(()),
        Some(client_email) => Err(Error::unsupported(
            email,
            format!("the key belongs to `{client_email}`"),
        )),
        None => Err(Error::unsupported(email, "the key names no `client_email`")),
    }
}

#[async_trait]
impl CredentialMinter for GcsDownscopedMinter {
    async fn mint(&self, request: &MintRequest) -> Result<VendedCredential> {
        if request.scope.location().provider() != StorageProvider::Gcp {
            return Err(Error::unsupported(
                &request.scope,
                "a GCP service account mints for gs:// storage only",
            ));
        }
        let boundary = access_boundary(&request.scope, request.access)?;
        let source = self.source_token(request.ttl).await?;
        let token = downscope_token(&source, &boundary, &self.options(&self.sts_endpoint)).await?;
        Ok(VendedCredential::new(
            StorageCredentials::GcpBearer(token.credential.bearer),
            request.scope.clone(),
            request.access,
        )
        .with_expiry(Some(token.expiration))
        .with_hints(self.hints()))
    }
}

impl fmt::Debug for GcsDownscopedMinter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match &self.source {
            Source::Impersonate(_) => "impersonate",
            Source::Key(_) => "key",
        };
        f.debug_struct("GcsDownscopedMinter")
            .field("email", &self.email)
            .field("source", &source)
            .field("iam_credentials_endpoint", &self.iam_credentials_endpoint)
            .field("token_endpoint", &self.token_endpoint)
            .field("sts_endpoint", &self.sts_endpoint)
            .field("gcs_endpoint", &self.gcs_endpoint)
            .field("max_ttl", &self.max_ttl)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{TimeDelta, Utc};
    use mockito::Matcher;
    use olai_http::StaticCredentialProvider;
    use olai_http::gcp::GcpCredential;
    use olai_storage_credentials::AccessLevel;
    use serde_json::json;

    use super::*;

    const EMAIL: &str = "vend@project.iam.gserviceaccount.com";

    fn impersonated(server: &mockito::Server) -> GcsDownscopedMinter {
        GcsDownscopedMinter::new(&GcpServiceAccount {
            email: EMAIL.into(),
            key: None,
        })
        .unwrap()
        .with_base_credentials(Arc::new(StaticCredentialProvider::new(GcpCredential {
            bearer: "host-token".into(),
        })))
        .with_iam_credentials_endpoint(server.url())
        .with_sts_endpoint(format!("{}/v1/token", server.url()))
        .with_client_options(ClientOptions::new().with_allow_http(true))
    }

    /// Mocks the impersonation call (expecting `lifetime`) and the STS exchange
    /// (expecting the boundary for `scope` at `access`).
    async fn mock_google(
        server: &mut mockito::Server,
        lifetime: &str,
        scope: &str,
        access: AccessLevel,
    ) -> (mockito::Mock, mockito::Mock) {
        let source_expiry = Utc::now() + TimeDelta::minutes(30);
        let iam = server
            .mock(
                "POST",
                format!("/v1/projects/-/serviceAccounts/{EMAIL}:generateAccessToken").as_str(),
            )
            .match_header("authorization", "Bearer host-token")
            .match_body(Matcher::PartialJson(json!({ "lifetime": lifetime })))
            .with_body(
                json!({
                    "accessToken": "sa-token",
                    "expireTime": source_expiry.to_rfc3339(),
                })
                .to_string(),
            )
            .create_async()
            .await;
        let boundary = access_boundary(&scope.parse().unwrap(), access).unwrap();
        let sts = server
            .mock("POST", "/v1/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("subject_token".into(), "sa-token".into()),
                Matcher::UrlEncoded("options".into(), boundary.to_string()),
            ]))
            .with_body(r#"{"access_token":"downscoped-token","expires_in":3600}"#)
            .create_async()
            .await;
        (iam, sts)
    }

    #[tokio::test]
    async fn mints_downscoped_tokens_for_the_scope() {
        for access in [AccessLevel::Read, AccessLevel::ReadWrite] {
            let mut server = mockito::Server::new_async().await;
            let scope = "gs://bucket/warehouse/tab";
            let (iam, sts) = mock_google(&mut server, "900s", scope, access).await;
            let request = MintRequest::new(scope.parse().unwrap(), access)
                .with_ttl(Duration::from_secs(900))
                .with_session_name("ignored");
            let vended = impersonated(&server).mint(&request).await.unwrap();
            iam.assert_async().await;
            sts.assert_async().await;

            assert_eq!(
                vended.credentials,
                StorageCredentials::GcpBearer("downscoped-token".into())
            );
            assert_eq!(vended.scope, request.scope);
            assert_eq!(vended.access, access);
            // STS said an hour, but the source token lives thirty minutes.
            let expiry = vended.expires_at.unwrap();
            assert!(expiry <= Utc::now() + TimeDelta::minutes(30), "{expiry}");
            assert!(vended.hints.is_empty());
        }
    }

    #[tokio::test]
    async fn lifetimes_are_clamped() {
        for (ttl, max_ttl, lifetime) in [
            (None, None, "3600s"),
            (Some(10 * 3600), None, "3600s"),
            (Some(10 * 3600), Some(4 * 3600), "14400s"),
            (Some(1), None, "60s"),
        ] {
            let mut server = mockito::Server::new_async().await;
            let scope = "gs://bucket/tab";
            let (iam, _sts) = mock_google(&mut server, lifetime, scope, AccessLevel::Read).await;
            let mut minter = impersonated(&server);
            if let Some(max_ttl) = max_ttl {
                minter = minter.with_max_ttl(Duration::from_secs(max_ttl));
            }
            let mut request = MintRequest::new(scope.parse().unwrap(), AccessLevel::Read);
            if let Some(ttl) = ttl {
                request = request.with_ttl(Duration::from_secs(ttl));
            }
            minter.mint(&request).await.unwrap();
            iam.assert_async().await;
        }
    }

    #[tokio::test]
    async fn rejects_other_clouds() {
        let server = mockito::Server::new_async().await;
        let request = MintRequest::new("s3://bucket/tab".parse().unwrap(), AccessLevel::Read);
        let err = impersonated(&server).mint(&request).await.unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[tokio::test]
    async fn the_gcs_endpoint_is_a_store_hint() {
        let mut server = mockito::Server::new_async().await;
        let scope = "gs://bucket/tab";
        let _mocks = mock_google(&mut server, "3600s", scope, AccessLevel::Read).await;
        let vended = impersonated(&server)
            .with_gcs_endpoint("http://127.0.0.1:4443")
            .mint(&MintRequest::new(scope.parse().unwrap(), AccessLevel::Read))
            .await
            .unwrap();
        assert_eq!(
            vended.hints.endpoint.as_deref(),
            Some("http://127.0.0.1:4443")
        );
        assert_eq!(vended.hints.allow_http, Some(true));
    }

    #[test]
    fn keys_must_belong_to_the_account() {
        let key = |email: &str| {
            Some(GcpServiceAccountKey::ServiceAccountKey(
                json!({ "type": "service_account", "client_email": email, "private_key": "SECRET-PEM" })
                    .to_string(),
            ))
        };
        let err = GcsDownscopedMinter::new(&GcpServiceAccount {
            email: EMAIL.into(),
            key: key("other@project.iam.gserviceaccount.com"),
        })
        .unwrap_err();
        assert!(err.to_string().contains("other@project"), "{err}");
        assert!(!err.to_string().contains("SECRET"), "{err}");

        let missing = GcsDownscopedMinter::new(&GcpServiceAccount {
            email: EMAIL.into(),
            key: Some(GcpServiceAccountKey::ServiceAccountKeyFile(
                "/nonexistent/sa.json".into(),
            )),
        })
        .unwrap_err();
        assert!(
            missing.to_string().contains("/nonexistent/sa.json"),
            "{missing}"
        );

        let minter = GcsDownscopedMinter::new(&GcpServiceAccount {
            email: EMAIL.into(),
            key: key(EMAIL),
        })
        .unwrap();
        let debug = format!("{minter:?}");
        assert!(debug.contains("\"key\""), "{debug}");
        assert!(!debug.contains("SECRET"), "{debug}");
    }
}
