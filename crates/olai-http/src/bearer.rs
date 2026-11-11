//! Bearer-token authentication for arbitrary HTTP APIs.
//!
//! The per-cloud and Databricks modules sign requests for one specific
//! service. This module builds bearer tokens for *any* audience: a provider is
//! configured with the token endpoint, scope, or audience of the API it is
//! for, so the same flows work against an API fronted by Entra ID, Okta,
//! Keycloak, Cognito, Google, or a self-issued token service.
//!
//! [`BearerAuth`] holds the HTTP settings shared by every token request and
//! creates [`BearerCredentialProvider`]s:
//!
//! | Flow | Constructor |
//! |------|-------------|
//! | Static token | [`BearerAuth::static_token`] |
//! | OAuth 2.0 client credentials against any token endpoint | [`BearerAuth::client_credentials`] |
//! | Token read from an environment variable / a file (e.g. a projected Kubernetes token) | [`BearerAuth::token_env`], [`BearerAuth::token_file`] |
//! | Azure service principal, managed identity, workload identity, Azure CLI | [`BearerAuth::azure_client_secret`], [`BearerAuth::azure_managed_identity`], [`BearerAuth::azure_workload_identity`], [`BearerAuth::azure_cli`] |
//! | GCP ID token from the metadata server, a service-account key, or the Google Cloud CLI | [`BearerAuth::gcp_id_token`], [`BearerAuth::gcp_service_account_id_token`], [`BearerAuth::gcloud_id_token`] |
//! | AWS identity as a signed `sts:GetCallerIdentity` request | [`BearerAuth::aws_signed_identity`] |
//! | RFC 8693 token exchange of any of the above | [`BearerAuth::token_exchange`] |
//!
//! Hand a provider to [`CloudClient::new_with_bearer`] to sign requests with it.
//!
//! [`CloudClient::new_with_bearer`]: crate::CloudClient::new_with_bearer

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HOST, HeaderName, HeaderValue};
use reqwest::{Client, Method, Request};
use serde::Deserialize;
use tokio::runtime::Handle;

use crate::aws::AwsCredentialProvider;
use crate::aws::credential::AwsAuthorizer;
use crate::azure::credential::{
    AzureCliCredential, AzureCredential, ClientSecretOAuthProvider, ImdsManagedIdentityProvider,
    WorkloadIdentityOAuthProvider,
};
use crate::error::generic;
use crate::gcp::credential::{ServiceAccountCredentials, ServiceAccountKey};
use crate::retry::RetryExt;
use crate::service::{HttpService, make_service};
use crate::token::jwt_exp_secs;
use crate::{
    ClientOptions, CredentialProvider, Result, RetryConfig, StaticCredentialProvider,
    TemporaryToken, TokenCache, TokenCredentialProvider, TokenProvider,
};

/// How long a token without a known expiry is used before it is fetched again.
const UNKNOWN_EXPIRY: Duration = Duration::from_secs(10 * 60);

/// RFC 8693 token-exchange grant type.
pub(crate) const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";

/// RFC 8693 token type of an OpenID Connect ID token.
pub const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";

/// RFC 8693 token type of an OAuth 2.0 access token.
pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

/// RFC 8693 token type of a JWT.
pub const JWT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";

/// RFC 7523 JWT-bearer grant type.
pub(crate) const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// The token endpoint of Google service accounts that do not name one.
const GCP_DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

const GCP_METADATA_IDENTITY_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity";

/// The body of a signed `sts:GetCallerIdentity` request.
pub const GET_CALLER_IDENTITY: &str = "Action=GetCallerIdentity&Version=2011-06-15";

/// How long STS takes a signed request after it is signed.
const STS_VALIDITY: Duration = Duration::from_secs(15 * 60);

/// How long a signed request is used: until [`STS_VALIDITY`] has this much left.
const AWS_SIGNED_MIN_TTL: Duration = Duration::from_secs(10 * 60);

/// A bearer token, sent as `Authorization: Bearer <token>`.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerCredential {
    /// The token. This is secret material.
    pub token: String,
}

impl fmt::Debug for BearerCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BearerCredential")
            .field("token", &"***")
            .finish()
    }
}

/// A shared provider of [`BearerCredential`]s.
pub type BearerCredentialProvider = Arc<dyn CredentialProvider<Credential = BearerCredential>>;

/// OAuth 2.0 client-credentials grant configuration.
#[derive(Clone, Default)]
pub struct ClientCredentials {
    /// The token endpoint, e.g. `https://login.example.com/oauth2/token`.
    pub token_url: String,
    /// The client id.
    pub client_id: String,
    /// The client secret.
    pub client_secret: String,
    /// Space-separated scopes to request, if the issuer needs them.
    pub scope: Option<String>,
    /// The audience (resource) to request, if the issuer needs it (e.g. Auth0).
    pub audience: Option<String>,
}

impl fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientCredentials")
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"***")
            .field("scope", &self.scope)
            .field("audience", &self.audience)
            .finish()
    }
}

/// RFC 8693 token-exchange configuration.
#[derive(Clone)]
pub struct TokenExchange {
    /// The token-exchange endpoint.
    pub endpoint: String,
    /// The provider of the token to exchange.
    pub subject: BearerCredentialProvider,
    /// The type of the subject token; defaults to [`ID_TOKEN_TYPE`].
    pub subject_token_type: Option<String>,
    /// The type of token to request; defaults to [`ACCESS_TOKEN_TYPE`].
    pub requested_token_type: Option<String>,
    /// The audience to request, if the exchange needs it.
    pub audience: Option<String>,
    /// Space-separated scopes to request, if the exchange needs them.
    pub scope: Option<String>,
}

impl fmt::Debug for TokenExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenExchange")
            .field("endpoint", &self.endpoint)
            .field("subject", &self.subject)
            .field("subject_token_type", &self.subject_token_type)
            .field("requested_token_type", &self.requested_token_type)
            .field("audience", &self.audience)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Azure managed-identity configuration.
#[derive(Debug, Clone, Default)]
pub struct AzureManagedIdentity {
    /// The resource to request a token for, e.g. `api://my-app`.
    pub resource: String,
    /// The client id of a user-assigned identity.
    pub client_id: Option<String>,
    /// The object id of a user-assigned identity.
    pub object_id: Option<String>,
    /// The Azure resource id of a user-assigned identity.
    pub msi_resource_id: Option<String>,
    /// Overrides the instance metadata token endpoint.
    pub endpoint: Option<String>,
}

/// A GCP service-account key.
#[derive(Clone)]
pub enum GcpServiceAccount {
    /// The path of a service-account JSON key file, e.g. the one named by
    /// `GOOGLE_APPLICATION_CREDENTIALS`.
    File(String),
    /// The contents of a service-account JSON key file. This is secret
    /// material.
    Key(String),
}

/// A [`GcpServiceAccount`] read and parsed.
pub(crate) struct LoadedServiceAccount {
    pub(crate) credentials: ServiceAccountCredentials,
    pub(crate) key: ServiceAccountKey,
    /// The key's token endpoint, or Google's.
    pub(crate) token_uri: String,
}

impl GcpServiceAccount {
    /// Reads and parses the key. Fails unless it is a `service_account` credential;
    /// `purpose` names what needs one, for the error.
    pub(crate) fn load(&self, purpose: &str) -> Result<LoadedServiceAccount> {
        let json = match self {
            Self::File(path) => std::fs::read_to_string(path).map_err(|e| {
                generic(format!("failed to read service-account key `{path}`: {e}"))
            })?,
            Self::Key(key) => key.clone(),
        };
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| generic(format!("invalid service-account key {self:?}: {e}")))?;
        if let Some(kind) = value.get("type").and_then(|kind| kind.as_str())
            && kind != "service_account"
        {
            return Err(generic(format!(
                "{self:?} holds `{kind}` credentials; {purpose} need a service-account key"
            )));
        }
        let credentials: ServiceAccountCredentials = serde_json::from_value(value)
            .map_err(|e| generic(format!("invalid service-account key {self:?}: {e}")))?;
        let key = ServiceAccountKey::from_pem(credentials.private_key.as_bytes())?;
        let token_uri = credentials
            .token_uri
            .clone()
            .unwrap_or_else(|| GCP_DEFAULT_TOKEN_URI.to_string());
        Ok(LoadedServiceAccount {
            credentials,
            key,
            token_uri,
        })
    }
}

impl fmt::Debug for GcpServiceAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            Self::Key(_) => f.debug_tuple("Key").field(&"***").finish(),
        }
    }
}

/// An AWS identity as a bearer token, for an API that has no token of AWS to check.
///
/// Each token is an `sts:GetCallerIdentity` request, signed with SigV4 but not sent: the
/// API sends it to STS, which names the caller. The credentials never leave the client.
/// The token is `prefix` followed by the request as unpadded base64url JSON:
/// `{"method": "POST", "url", "headers", "body"}`, with lowercase header names and without
/// `host`, which the API's HTTP client sets itself.
#[derive(Clone)]
pub struct AwsSignedIdentity {
    /// The credentials that sign, e.g. [`AmazonConfig::credentials`].
    ///
    /// [`AmazonConfig::credentials`]: crate::aws::AmazonConfig::credentials
    pub credentials: AwsCredentialProvider,
    /// The STS endpoint, e.g. `https://sts.us-east-1.amazonaws.com/`.
    pub endpoint: String,
    /// The region that signs: the region of `endpoint`, `us-east-1` for the global one.
    pub region: String,
    /// More headers to sign, such as one that names the API, so that another API that
    /// receives the token cannot use it. Names are lowercased.
    pub headers: Vec<(String, String)>,
    /// What goes before the encoded request, so that the API can tell the token apart.
    pub prefix: String,
}

impl fmt::Debug for AwsSignedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsSignedIdentity")
            .field("credentials", &self.credentials)
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("headers", &self.headers)
            .field("prefix", &self.prefix)
            .finish()
    }
}

/// Builds [`BearerCredentialProvider`]s that share HTTP settings.
///
/// Providers that fetch tokens over HTTP use the configured
/// [`ClientOptions`] and [`RetryConfig`], and spawn their requests on the
/// runtime set with [`with_runtime`](Self::with_runtime). All of them cache
/// tokens and refresh them before they expire.
#[derive(Debug, Clone, Default)]
pub struct BearerAuth {
    client_options: ClientOptions,
    retry: RetryConfig,
    runtime: Option<Handle>,
}

impl BearerAuth {
    /// Create a builder with default HTTP settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the HTTP client options used for token requests.
    pub fn with_client_options(mut self, options: ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// Set the retry configuration used for token requests.
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Spawn token requests on `handle`.
    pub fn with_runtime(mut self, handle: Handle) -> Self {
        self.runtime = Some(handle);
        self
    }

    /// A provider that always returns `token`.
    pub fn static_token(token: impl Into<String>) -> BearerCredentialProvider {
        Arc::new(StaticCredentialProvider::new(BearerCredential {
            token: token.into(),
        }))
    }

    /// OAuth 2.0 client credentials against any token endpoint.
    pub fn client_credentials(
        &self,
        config: ClientCredentials,
    ) -> Result<BearerCredentialProvider> {
        self.fetching(ClientCredentialsProvider(config), false)
    }

    /// A token read from the environment variable `var` on every refresh.
    ///
    /// A JWT is reused until shortly before its `exp`; other tokens are re-read
    /// every few minutes.
    pub fn token_env(&self, var: impl Into<String>) -> BearerCredentialProvider {
        Arc::new(LocalTokenProvider::new(TokenLocation::Env(var.into())))
    }

    /// A token read from the file at `path` on every refresh, e.g. a projected
    /// Kubernetes service-account token that rotates in place.
    ///
    /// A JWT is reused until shortly before its `exp`; other tokens are re-read
    /// every few minutes.
    pub fn token_file(&self, path: impl Into<String>) -> BearerCredentialProvider {
        Arc::new(LocalTokenProvider::new(TokenLocation::File(path.into())))
    }

    /// An Azure AD token for `scope` (e.g. `api://my-app/.default`) from a
    /// service principal's client secret.
    pub fn azure_client_secret(
        &self,
        tenant_id: &str,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        scope: &str,
        authority_host: Option<String>,
    ) -> Result<BearerCredentialProvider> {
        let provider = ClientSecretOAuthProvider::new_with_scope(
            client_id.into(),
            client_secret.into(),
            tenant_id,
            authority_host,
            scope,
        );
        Ok(azure_bearer(self.azure_fetching(provider, false)?))
    }

    /// An Azure AD token from the managed identity of the host, optionally a
    /// user-assigned one.
    pub fn azure_managed_identity(
        &self,
        identity: AzureManagedIdentity,
    ) -> Result<BearerCredentialProvider> {
        let provider = ImdsManagedIdentityProvider::new_with_resource(
            identity.client_id,
            identity.object_id,
            identity.msi_resource_id,
            identity.endpoint,
            &identity.resource,
        );
        Ok(azure_bearer(self.azure_fetching(provider, true)?))
    }

    /// An Azure AD token for `scope` from a federated token file, as used by
    /// AKS workload identity (`AZURE_FEDERATED_TOKEN_FILE`).
    pub fn azure_workload_identity(
        &self,
        tenant_id: &str,
        client_id: impl Into<String>,
        federated_token_file: impl Into<String>,
        scope: &str,
        authority_host: Option<String>,
    ) -> Result<BearerCredentialProvider> {
        let provider = WorkloadIdentityOAuthProvider::new(
            client_id,
            federated_token_file,
            tenant_id,
            authority_host,
        )
        .with_scope(scope);
        Ok(azure_bearer(self.azure_fetching(provider, false)?))
    }

    /// An Azure AD token for `scope` from the signed-in Azure CLI
    /// (`az account get-access-token`).
    pub fn azure_cli(&self, scope: impl Into<String>) -> BearerCredentialProvider {
        azure_bearer(Arc::new(AzureCliCredential::new_with_scope(scope)))
    }

    /// An ID token for `audience` from the GCP metadata server of the host's
    /// service account.
    ///
    /// `endpoint` overrides the metadata identity endpoint.
    pub fn gcp_id_token(
        &self,
        audience: impl Into<String>,
        endpoint: Option<String>,
    ) -> Result<BearerCredentialProvider> {
        let provider = GcpIdTokenProvider {
            audience: audience.into(),
            endpoint: endpoint.unwrap_or_else(|| GCP_METADATA_IDENTITY_URL.to_string()),
        };
        self.fetching(provider, true)
    }

    /// An ID token for `audience`, signed for by a service-account key.
    ///
    /// The key must be a `service_account` credential: user credentials from
    /// `gcloud auth application-default login` cannot mint ID tokens for an
    /// arbitrary audience.
    pub fn gcp_service_account_id_token(
        &self,
        account: GcpServiceAccount,
        audience: impl Into<String>,
    ) -> Result<BearerCredentialProvider> {
        let LoadedServiceAccount {
            credentials,
            key,
            token_uri,
        } = account.load("ID tokens")?;
        let provider = GcpServiceAccountIdTokenProvider {
            credentials,
            key,
            token_uri,
            audience: audience.into(),
        };
        self.fetching(provider, false)
    }

    /// An ID token for `audience` from the sign-in of the Google Cloud CLI
    /// (`gcloud auth print-identity-token`).
    ///
    /// A person's sign-in cannot ask for an audience: when the CLI refuses one, the token
    /// is asked for without it.
    pub fn gcloud_id_token(&self, audience: impl Into<String>) -> BearerCredentialProvider {
        Arc::new(GcloudIdTokenProvider::new(GCLOUD, audience.into()))
    }

    /// The AWS identity of `config.credentials` as signed `sts:GetCallerIdentity` requests
    /// ([`AwsSignedIdentity`]). A request is signed again when its 15 minutes at STS have
    /// 10 left.
    pub fn aws_signed_identity(
        &self,
        config: AwsSignedIdentity,
    ) -> Result<BearerCredentialProvider> {
        let endpoint = url::Url::parse(&config.endpoint)
            .map_err(|e| generic(format!("invalid STS endpoint `{}`: {e}", config.endpoint)))?;
        Ok(Arc::new(AwsSignedIdentityProvider {
            config,
            endpoint,
            cache: TokenCache::default().with_min_ttl(AWS_SIGNED_MIN_TTL),
        }))
    }

    /// Exchange the subject token for another token (RFC 8693), e.g. an
    /// identity-provider ID token for an API's own access token.
    pub fn token_exchange(&self, config: TokenExchange) -> Result<BearerCredentialProvider> {
        self.fetching(TokenExchangeProvider(config), false)
    }

    fn http(&self, metadata: bool) -> Result<(Client, Arc<dyn HttpService>)> {
        let client = if metadata {
            self.client_options.metadata_client()?
        } else {
            self.client_options.client()?
        };
        let service = make_service(client.clone(), self.runtime.as_ref());
        Ok((client, service))
    }

    fn fetching<T>(&self, provider: T, metadata: bool) -> Result<BearerCredentialProvider>
    where
        T: TokenProvider<Credential = BearerCredential> + 'static,
    {
        let (client, service) = self.http(metadata)?;
        Ok(Arc::new(TokenCredentialProvider::new(
            provider,
            client,
            service,
            self.retry.clone(),
        )))
    }

    fn azure_fetching<T>(
        &self,
        provider: T,
        metadata: bool,
    ) -> Result<Arc<dyn CredentialProvider<Credential = AzureCredential>>>
    where
        T: TokenProvider<Credential = AzureCredential> + 'static,
    {
        let (client, service) = self.http(metadata)?;
        Ok(Arc::new(TokenCredentialProvider::new(
            provider,
            client,
            service,
            self.retry.clone(),
        )))
    }
}

/// Signs requests with a bearer token from a [`BearerCredentialProvider`], for
/// [`CloudClient::new_with_request_signer`] and signers that wrap it.
///
/// [`CloudClient::new_with_request_signer`]: crate::CloudClient::new_with_request_signer
#[derive(Debug)]
pub struct BearerProviderSigner(pub BearerCredentialProvider);

impl crate::RequestSigner for BearerProviderSigner {
    fn sign<'a>(
        &'a self,
        req: reqwest::RequestBuilder,
    ) -> futures::future::BoxFuture<'a, Result<reqwest::RequestBuilder>> {
        Box::pin(async move {
            let credential = self.0.get_credential().await?;
            Ok(req.bearer_auth(&credential.token))
        })
    }
}

fn azure_bearer(
    inner: Arc<dyn CredentialProvider<Credential = AzureCredential>>,
) -> BearerCredentialProvider {
    Arc::new(AzureBearer(inner))
}

/// Serves an Azure AD bearer token as a [`BearerCredential`].
#[derive(Debug)]
struct AzureBearer(Arc<dyn CredentialProvider<Credential = AzureCredential>>);

#[async_trait]
impl CredentialProvider for AzureBearer {
    type Credential = BearerCredential;

    async fn get_credential(&self) -> Result<Arc<BearerCredential>> {
        let credential = self.0.get_credential().await?;
        let AzureCredential::BearerToken(token) = credential.as_ref();
        Ok(Arc::new(BearerCredential {
            token: token.clone(),
        }))
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl TokenResponse {
    fn into_token(self) -> TemporaryToken<Arc<BearerCredential>> {
        let expiry = match self.expires_in {
            Some(seconds) => Instant::now() + Duration::from_secs(seconds),
            None => {
                jwt_expiry(&self.access_token).unwrap_or_else(|| Instant::now() + UNKNOWN_EXPIRY)
            }
        };
        TemporaryToken {
            token: Arc::new(BearerCredential {
                token: self.access_token,
            }),
            expiry: Some(expiry),
        }
    }
}

#[derive(Debug)]
struct ClientCredentialsProvider(ClientCredentials);

#[async_trait]
impl TokenProvider for ClientCredentialsProvider {
    type Credential = BearerCredential;

    async fn fetch_token(
        &self,
        client: &Client,
        service: &Arc<dyn HttpService>,
        retry: &RetryConfig,
    ) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        let config = &self.0;
        let mut form = vec![
            ("grant_type", "client_credentials"),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ];
        if let Some(scope) = &config.scope {
            form.push(("scope", scope));
        }
        if let Some(audience) = &config.audience {
            form.push(("audience", audience));
        }
        let response: TokenResponse = client
            .request(Method::POST, &config.token_url)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .form(&form)
            .retryable(retry, service.clone())
            .idempotent(true)
            .send()
            .await
            .map_err(|e| e.error())?
            .json()
            .await?;
        Ok(response.into_token())
    }
}

#[derive(Debug)]
struct TokenExchangeProvider(TokenExchange);

#[async_trait]
impl TokenProvider for TokenExchangeProvider {
    type Credential = BearerCredential;

    async fn fetch_token(
        &self,
        client: &Client,
        service: &Arc<dyn HttpService>,
        retry: &RetryConfig,
    ) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        let config = &self.0;
        let subject = config.subject.get_credential().await?;
        let mut form = vec![
            ("grant_type", TOKEN_EXCHANGE_GRANT),
            ("subject_token", subject.token.as_str()),
            (
                "subject_token_type",
                config
                    .subject_token_type
                    .as_deref()
                    .unwrap_or(ID_TOKEN_TYPE),
            ),
            (
                "requested_token_type",
                config
                    .requested_token_type
                    .as_deref()
                    .unwrap_or(ACCESS_TOKEN_TYPE),
            ),
        ];
        if let Some(audience) = &config.audience {
            form.push(("audience", audience));
        }
        if let Some(scope) = &config.scope {
            form.push(("scope", scope));
        }
        let response: TokenResponse = client
            .request(Method::POST, &config.endpoint)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .form(&form)
            .retryable(retry, service.clone())
            .idempotent(true)
            .send()
            .await
            .map_err(|e| e.error())?
            .json()
            .await?;
        Ok(response.into_token())
    }
}

#[derive(Debug)]
struct GcpIdTokenProvider {
    audience: String,
    endpoint: String,
}

#[async_trait]
impl TokenProvider for GcpIdTokenProvider {
    type Credential = BearerCredential;

    async fn fetch_token(
        &self,
        client: &Client,
        service: &Arc<dyn HttpService>,
        retry: &RetryConfig,
    ) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        let token = client
            .request(Method::GET, &self.endpoint)
            .header("Metadata-Flavor", "Google")
            .query(&[("audience", self.audience.as_str()), ("format", "full")])
            .send_retry(retry, service.clone())
            .await
            .map_err(|e| e.error())?
            .text()
            .await?
            .trim()
            .to_string();
        let expiry = jwt_expiry(&token).unwrap_or_else(|| Instant::now() + UNKNOWN_EXPIRY);
        Ok(TemporaryToken {
            token: Arc::new(BearerCredential { token }),
            expiry: Some(expiry),
        })
    }
}

struct GcpServiceAccountIdTokenProvider {
    credentials: ServiceAccountCredentials,
    key: ServiceAccountKey,
    token_uri: String,
    audience: String,
}

impl fmt::Debug for GcpServiceAccountIdTokenProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpServiceAccountIdTokenProvider")
            .field("client_email", &self.credentials.client_email)
            .field("token_uri", &self.token_uri)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl TokenProvider for GcpServiceAccountIdTokenProvider {
    type Credential = BearerCredential;

    async fn fetch_token(
        &self,
        client: &Client,
        service: &Arc<dyn HttpService>,
        retry: &RetryConfig,
    ) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        #[derive(Deserialize)]
        struct IdTokenResponse {
            id_token: String,
        }

        let assertion =
            self.credentials
                .id_token_assertion(&self.key, &self.token_uri, &self.audience)?;
        let response: IdTokenResponse = client
            .request(Method::POST, &self.token_uri)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .form(&[("grant_type", JWT_BEARER_GRANT), ("assertion", &assertion)])
            .retryable(retry, service.clone())
            .idempotent(true)
            .send()
            .await
            .map_err(|e| e.error())?
            .json()
            .await?;
        let token = response.id_token;
        let expiry = jwt_expiry(&token).unwrap_or_else(|| Instant::now() + UNKNOWN_EXPIRY);
        Ok(TemporaryToken {
            token: Arc::new(BearerCredential { token }),
            expiry: Some(expiry),
        })
    }
}

/// The Google Cloud CLI. On Windows it is a batch file, which `Command` finds only by its
/// full name.
const GCLOUD: &str = if cfg!(windows) {
    "gcloud.cmd"
} else {
    "gcloud"
};

/// The ID tokens of `gcloud auth print-identity-token`.
#[derive(Debug)]
struct GcloudIdTokenProvider {
    program: String,
    audience: String,
    cache: TokenCache<Arc<BearerCredential>>,
}

impl GcloudIdTokenProvider {
    fn new(program: impl Into<String>, audience: String) -> Self {
        Self {
            program: program.into(),
            audience,
            cache: Default::default(),
        }
    }

    /// Run the CLI, with the audience when `audience`.
    async fn print_identity_token(&self, audience: bool) -> Result<String> {
        let mut command = std::process::Command::new(&self.program);
        command.args(["auth", "print-identity-token"]);
        if audience {
            command.arg("--audiences").arg(&self.audience);
        }
        let output = tokio::task::spawn_blocking(move || command.output())
            .await
            .map_err(|e| generic(format!("cannot run gcloud: {e}")))?
            .map_err(|e| generic(format!("cannot run gcloud: {e}")))?;
        if !output.status.success() {
            let reason = String::from_utf8_lossy(&output.stderr);
            return Err(generic(format!(
                "gcloud auth print-identity-token: {}",
                reason.trim()
            )));
        }
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match token.is_empty() {
            true => Err(generic("gcloud printed no token")),
            false => Ok(token),
        }
    }

    async fn fetch(&self) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        let token = match self.print_identity_token(true).await {
            Ok(token) => token,
            Err(with_audience) => self
                .print_identity_token(false)
                .await
                .map_err(|_| with_audience)?,
        };
        let expiry = jwt_expiry(&token).unwrap_or_else(|| Instant::now() + UNKNOWN_EXPIRY);
        Ok(TemporaryToken {
            token: Arc::new(BearerCredential { token }),
            expiry: Some(expiry),
        })
    }
}

#[async_trait]
impl CredentialProvider for GcloudIdTokenProvider {
    type Credential = BearerCredential;

    async fn get_credential(&self) -> Result<Arc<BearerCredential>> {
        self.cache.get_or_insert_with(|| self.fetch()).await
    }
}

/// The signed `sts:GetCallerIdentity` requests of an [`AwsSignedIdentity`].
#[derive(Debug)]
struct AwsSignedIdentityProvider {
    config: AwsSignedIdentity,
    endpoint: url::Url,
    cache: TokenCache<Arc<BearerCredential>>,
}

impl AwsSignedIdentityProvider {
    async fn sign(&self) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        use base64::Engine;

        let credential = self.config.credentials.get_credential().await?;
        let mut request = Request::new(Method::POST, self.endpoint.clone());
        let headers = request.headers_mut();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded; charset=utf-8"),
        );
        for (name, value) in &self.config.headers {
            let name = HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes())
                .map_err(|e| generic(format!("invalid header name `{name}`: {e}")))?;
            headers.insert(name, HeaderValue::from_str(value)?);
        }
        *request.body_mut() = Some(GET_CALLER_IDENTITY.into());
        AwsAuthorizer::new(&credential, "sts", &self.config.region)
            .authorize(&mut request, None)?;
        let signed: std::collections::BTreeMap<&str, String> = (request.headers().iter())
            .filter(|(name, _)| *name != HOST)
            .map(|(name, value)| {
                (
                    name.as_str(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let json = serde_json::json!({
            "method": "POST",
            "url": self.endpoint.as_str(),
            "headers": signed,
            "body": GET_CALLER_IDENTITY,
        });
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.to_string());
        Ok(TemporaryToken {
            token: Arc::new(BearerCredential {
                token: format!("{}{encoded}", self.config.prefix),
            }),
            expiry: Some(Instant::now() + STS_VALIDITY),
        })
    }
}

#[async_trait]
impl CredentialProvider for AwsSignedIdentityProvider {
    type Credential = BearerCredential;

    async fn get_credential(&self) -> Result<Arc<BearerCredential>> {
        self.cache.get_or_insert_with(|| self.sign()).await
    }
}

#[derive(Debug)]
enum TokenLocation {
    Env(String),
    File(String),
}

/// Reads a token from the environment or a file, re-reading it near expiry.
#[derive(Debug)]
struct LocalTokenProvider {
    location: TokenLocation,
    cache: TokenCache<Arc<BearerCredential>>,
}

impl LocalTokenProvider {
    fn new(location: TokenLocation) -> Self {
        Self {
            location,
            cache: Default::default(),
        }
    }

    fn read(&self) -> Result<TemporaryToken<Arc<BearerCredential>>> {
        let token = match &self.location {
            TokenLocation::Env(var) => std::env::var(var)
                .map_err(|_| generic(format!("environment variable `{var}` is not set")))?,
            TokenLocation::File(path) => std::fs::read_to_string(path)
                .map_err(|e| generic(format!("failed to read token file `{path}`: {e}")))?,
        };
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err(generic(format!(
                "the token in {:?} is empty",
                self.location
            )));
        }
        let expiry = jwt_expiry(&token).unwrap_or_else(|| Instant::now() + UNKNOWN_EXPIRY);
        Ok(TemporaryToken {
            token: Arc::new(BearerCredential { token }),
            expiry: Some(expiry),
        })
    }
}

#[async_trait]
impl CredentialProvider for LocalTokenProvider {
    type Credential = BearerCredential;

    async fn get_credential(&self) -> Result<Arc<BearerCredential>> {
        self.cache
            .get_or_insert_with(|| async { self.read() })
            .await
    }
}

/// The `exp` of a JWT as an [`Instant`], if `token` is a JWT with one.
fn jwt_expiry(token: &str) -> Option<Instant> {
    let exp = jwt_exp_secs(token)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Instant::now() + Duration::from_secs(exp.saturating_sub(now)))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use mockito::Matcher;

    use super::*;
    use crate::{ClientConfigKey, CloudClient};

    fn auth() -> BearerAuth {
        BearerAuth::new().with_client_options(
            ClientOptions::new().with_config(ClientConfigKey::AllowHttp, "true"),
        )
    }

    /// A JWT (unsigned) whose payload is `{"exp": <exp>}`.
    fn jwt(exp: u64) -> String {
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("eyJhbGciOiJub25lIn0.{payload}.")
    }

    fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// Send one request through a client signed by `provider` and assert the
    /// bearer it carried.
    async fn assert_signs(
        server: &mut mockito::Server,
        provider: BearerCredentialProvider,
        token: &str,
    ) {
        let api = server
            .mock("GET", "/api")
            .match_header("authorization", format!("Bearer {token}").as_str())
            .with_status(200)
            .create_async()
            .await;
        let options = ClientOptions::new().with_config(ClientConfigKey::AllowHttp, "true");
        let client =
            CloudClient::new_with_bearer(provider, &options, RetryConfig::default(), None).unwrap();
        let status = client
            .get(format!("{}/api", server.url()))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 200);
        api.assert_async().await;
    }

    /// Signs with a bearer token only once it is switched on.
    #[derive(Debug)]
    struct Switched {
        on: std::sync::atomic::AtomicBool,
        inner: BearerProviderSigner,
    }

    impl crate::RequestSigner for Switched {
        fn sign<'a>(
            &'a self,
            req: reqwest::RequestBuilder,
        ) -> futures::future::BoxFuture<'a, Result<reqwest::RequestBuilder>> {
            match self.on.load(std::sync::atomic::Ordering::SeqCst) {
                true => self.inner.sign(req),
                false => Box::pin(async move { Ok(req) }),
            }
        }
    }

    #[tokio::test]
    async fn any_request_signer_signs() {
        let mut server = mockito::Server::new_async().await;
        let without = server
            .mock("GET", "/api")
            .match_header("authorization", Matcher::Missing)
            .expect(1)
            .create_async()
            .await;
        let signer = Arc::new(Switched {
            on: false.into(),
            inner: BearerProviderSigner(BearerAuth::static_token("late-token")),
        });
        let options = ClientOptions::new().with_allow_http(true);
        let client = CloudClient::new_with_request_signer(
            signer.clone(),
            &options,
            RetryConfig::default(),
            None,
        )
        .unwrap();
        let url = format!("{}/api", server.url());
        assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
        without.assert_async().await;

        signer.on.store(true, std::sync::atomic::Ordering::SeqCst);
        let with = server
            .mock("GET", "/api")
            .match_header("authorization", "Bearer late-token")
            .expect(1)
            .create_async()
            .await;
        assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
        with.assert_async().await;
    }

    #[tokio::test]
    async fn aws_signed_identity_signs_get_caller_identity() {
        use crate::aws::AwsCredential;

        let credentials: AwsCredentialProvider =
            Arc::new(StaticCredentialProvider::new(AwsCredential {
                key_id: "AKIDEXAMPLE".into(),
                secret_key: "secret".into(),
                token: Some("session".into()),
            }));
        let provider = auth()
            .aws_signed_identity(AwsSignedIdentity {
                credentials,
                endpoint: "https://sts.eu-west-1.amazonaws.com/".into(),
                region: "eu-west-1".into(),
                headers: vec![("X-Api-Id".into(), "https://api.example.com".into())],
                prefix: "api-aws-v1.".into(),
            })
            .unwrap();
        let token = provider.get_credential().await.unwrap().token.clone();
        let encoded = token.strip_prefix("api-aws-v1.").unwrap();
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .unwrap();
        let signed: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(signed["method"], "POST");
        assert_eq!(signed["url"], "https://sts.eu-west-1.amazonaws.com/");
        assert_eq!(signed["body"], GET_CALLER_IDENTITY);
        let headers = &signed["headers"];
        assert_eq!(headers["x-api-id"], "https://api.example.com");
        assert_eq!(headers["x-amz-security-token"], "session");
        assert!(headers.get("host").is_none(), "{headers}");
        let authorization = headers["authorization"].as_str().unwrap();
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
                && authorization.contains("/eu-west-1/sts/aws4_request")
                && authorization.contains("x-api-id"),
            "{authorization}"
        );
        // Kept: the next call gives the same signed request.
        assert_eq!(provider.get_credential().await.unwrap().token, token);

        let invalid = auth().aws_signed_identity(AwsSignedIdentity {
            credentials: Arc::new(StaticCredentialProvider::new(AwsCredential {
                key_id: "k".into(),
                secret_key: "s".into(),
                token: None,
            })),
            endpoint: "not a url".into(),
            region: "us-east-1".into(),
            headers: vec![],
            prefix: String::new(),
        });
        assert!(invalid.is_err());
    }

    /// A stand-in for gcloud: prints `token` when asked with an audience, else
    /// `fallback`; fails when the one it prints is empty.
    #[cfg(unix)]
    fn fake_gcloud(dir: &std::path::Path, token: &str, fallback: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("gcloud");
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *--audiences*) t='{token}' ;;\n  *) t='{fallback}' ;;\nesac\n\
             [ -n \"$t\" ] || {{ echo 'no audience for you' >&2; exit 1; }}\necho \"$t\"\n"
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gcloud_id_token_asks_without_the_audience_when_refused() {
        let dir = tempfile::tempdir().unwrap();
        let with = jwt(unix_now() + 3600);
        let program = fake_gcloud(dir.path(), &with, "plain");
        let provider = GcloudIdTokenProvider::new(&program, "aud".into());
        assert_eq!(provider.get_credential().await.unwrap().token, with);

        let dir = tempfile::tempdir().unwrap();
        let program = fake_gcloud(dir.path(), "", "personal");
        let provider = GcloudIdTokenProvider::new(&program, "aud".into());
        assert_eq!(provider.get_credential().await.unwrap().token, "personal");

        let dir = tempfile::tempdir().unwrap();
        let program = fake_gcloud(dir.path(), "", "");
        let provider = GcloudIdTokenProvider::new(&program, "aud".into());
        let err = provider.get_credential().await.unwrap_err();
        assert!(err.to_string().contains("no audience for you"), "{err}");

        let missing = GcloudIdTokenProvider::new("/nonexistent/gcloud", "aud".into());
        assert!(missing.get_credential().await.is_err());
    }

    #[tokio::test]
    async fn client_credentials_against_any_token_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let token = server
            .mock("POST", "/oauth2/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "client_credentials".into()),
                Matcher::UrlEncoded("client_id".into(), "app".into()),
                Matcher::UrlEncoded("client_secret".into(), "secret".into()),
                Matcher::UrlEncoded("scope".into(), "catalog.read".into()),
                Matcher::UrlEncoded("audience".into(), "https://uc.example.com".into()),
            ]))
            .with_header("content-type", "application/json")
            .with_body(r#"{"access_token":"cc-token","expires_in":3600}"#)
            .expect(1)
            .create_async()
            .await;

        let provider = auth()
            .client_credentials(ClientCredentials {
                token_url: format!("{}/oauth2/token", server.url()),
                client_id: "app".into(),
                client_secret: "secret".into(),
                scope: Some("catalog.read".into()),
                audience: Some("https://uc.example.com".into()),
            })
            .unwrap();
        assert_signs(&mut server, provider, "cc-token").await;
        token.assert_async().await;
    }

    /// The subject token is exchanged per RFC 8693 and the issued token signs.
    #[tokio::test]
    async fn token_exchange_swaps_the_subject_token() {
        let mut server = mockito::Server::new_async().await;
        let exchanged = jwt(unix_now() + 3600);
        let exchange = server
            .mock("POST", "/api/1.0/unity-control/auth/tokens")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), TOKEN_EXCHANGE_GRANT.into()),
                Matcher::UrlEncoded("subject_token".into(), "idp-id-token".into()),
                Matcher::UrlEncoded("subject_token_type".into(), ID_TOKEN_TYPE.into()),
                Matcher::UrlEncoded("requested_token_type".into(), ACCESS_TOKEN_TYPE.into()),
            ]))
            .with_header("content-type", "application/json")
            // No `expires_in`: the JWT's `exp` decides when to refresh.
            .with_body(format!(
                r#"{{"access_token":"{exchanged}","token_type":"Bearer"}}"#
            ))
            .expect(1)
            .create_async()
            .await;

        let provider = auth()
            .token_exchange(TokenExchange {
                endpoint: format!("{}/api/1.0/unity-control/auth/tokens", server.url()),
                subject: BearerAuth::static_token("idp-id-token"),
                subject_token_type: None,
                requested_token_type: None,
                audience: None,
                scope: None,
            })
            .unwrap();
        // Two calls, one exchange: the issued token is cached until near `exp`.
        provider.get_credential().await.unwrap();
        assert_signs(&mut server, provider, &exchanged).await;
        exchange.assert_async().await;
    }

    #[tokio::test]
    async fn azure_client_secret_uses_the_given_scope() {
        let mut server = mockito::Server::new_async().await;
        let aad = server
            .mock("POST", "/tenant-1/oauth2/v2.0/token")
            .match_body(Matcher::UrlEncoded(
                "scope".into(),
                "api://my-catalog/.default".into(),
            ))
            .with_header("content-type", "application/json")
            .with_body(r#"{"access_token":"aad-token","expires_in":3600}"#)
            .create_async()
            .await;

        let provider = auth()
            .azure_client_secret(
                "tenant-1",
                "app",
                "secret",
                "api://my-catalog/.default",
                Some(server.url()),
            )
            .unwrap();
        assert_signs(&mut server, provider, "aad-token").await;
        aad.assert_async().await;
    }

    #[tokio::test]
    async fn azure_managed_identity_uses_the_given_resource() {
        let mut server = mockito::Server::new_async().await;
        let imds = server
            .mock("GET", "/metadata/identity/oauth2/token")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("resource".into(), "api://my-catalog".into()),
                Matcher::UrlEncoded("client_id".into(), "uami".into()),
            ]))
            .match_header("metadata", "true")
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"msi-token","expires_on":"{}","expires_in":"3600"}}"#,
                unix_now() + 3600
            ))
            .create_async()
            .await;

        let provider = auth()
            .azure_managed_identity(AzureManagedIdentity {
                resource: "api://my-catalog".into(),
                client_id: Some("uami".into()),
                endpoint: Some(format!("{}/metadata/identity/oauth2/token", server.url())),
                ..Default::default()
            })
            .unwrap();
        assert_signs(&mut server, provider, "msi-token").await;
        imds.assert_async().await;
    }

    #[test]
    fn gcp_service_account_id_token_rejects_user_credentials() {
        let key = r#"{"type":"authorized_user","client_id":"id","client_secret":"secret","refresh_token":"refresh"}"#;
        let err = auth()
            .gcp_service_account_id_token(GcpServiceAccount::Key(key.into()), "aud")
            .unwrap_err();
        assert!(err.to_string().contains("`authorized_user`"), "{err}");
    }

    #[test]
    fn gcp_service_account_id_token_validates_the_key() {
        let missing = auth()
            .gcp_service_account_id_token(
                GcpServiceAccount::File("/nonexistent.json".into()),
                "aud",
            )
            .unwrap_err();
        assert!(
            missing.to_string().contains("/nonexistent.json"),
            "{missing}"
        );

        let key = r#"{"type":"service_account","private_key":"not a key","private_key_id":"kid","client_email":"sa@example.iam.gserviceaccount.com"}"#;
        assert!(
            auth()
                .gcp_service_account_id_token(GcpServiceAccount::Key(key.into()), "aud")
                .is_err()
        );
        let debug = format!("{:?}", GcpServiceAccount::Key(key.into()));
        assert!(!debug.contains("private_key"), "{debug}");
    }

    #[tokio::test]
    async fn gcp_id_token_requests_the_audience() {
        let mut server = mockito::Server::new_async().await;
        let id_token = jwt(unix_now() + 3600);
        let metadata = server
            .mock("GET", "/identity")
            .match_query(Matcher::UrlEncoded(
                "audience".into(),
                "https://uc.example.com".into(),
            ))
            .match_header("metadata-flavor", "Google")
            .with_body(&id_token)
            .create_async()
            .await;

        let provider = auth()
            .gcp_id_token(
                "https://uc.example.com",
                Some(format!("{}/identity", server.url())),
            )
            .unwrap();
        assert_signs(&mut server, provider, &id_token).await;
        metadata.assert_async().await;
    }

    #[tokio::test]
    async fn token_file_is_trimmed_and_missing_files_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "file-token\n").unwrap();

        let provider = auth().token_file(path.to_string_lossy());
        assert_eq!(provider.get_credential().await.unwrap().token, "file-token");

        let missing = auth().token_file(dir.path().join("absent").to_string_lossy());
        assert!(missing.get_credential().await.is_err());
    }

    #[test]
    fn jwt_expiry_reads_exp_and_ignores_opaque_tokens() {
        let in_an_hour = jwt_expiry(&jwt(unix_now() + 3600)).unwrap();
        let remaining = in_an_hour.saturating_duration_since(Instant::now());
        assert!(remaining > Duration::from_secs(3500) && remaining <= Duration::from_secs(3600));
        assert!(jwt_expiry("opaque-token").is_none());
        assert!(jwt_expiry("a.not-base64!.c").is_none());
    }

    #[test]
    fn debug_redacts_secrets() {
        let credential = BearerCredential {
            token: "super-secret".into(),
        };
        let config = ClientCredentials {
            client_secret: "super-secret".into(),
            ..Default::default()
        };
        for debug in [format!("{credential:?}"), format!("{config:?}")] {
            assert!(!debug.contains("super-secret"), "secret leaked: {debug}");
        }
    }
}
