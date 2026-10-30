//! Authentication for vending servers, configured from string keys (feature `auth`,
//! native only).
//!
//! Every protocol module authenticates the same ways, so the keys are shared and only
//! their prefix differs: the `unity` module reads `unity_token`, `unity_client_id`, ...
//! An [`AuthConfig`] collects them, together with the well-known Azure and Google
//! identity keys a deployment provisions, and builds the authenticated olai-http
//! [`CloudClient`] for the server.
//!
//! | Key (after the prefix) | Meaning |
//! |---|---|
//! | `_auth_type` | Force an [`AuthType`]; detected when unset |
//! | `_allow_unauthenticated` | Send requests unauthenticated when nothing is configured |
//! | `_token`, `_access_token` | A static bearer token |
//! | `_token_file`, `_token_env` | A token re-read from a file or variable on refresh |
//! | `_client_id`, `_client_secret` | OAuth client credentials, or an Azure service principal |
//! | `_oauth_token_url`, `_oauth_scope`, `_oauth_audience` | OAuth endpoint, scope and audience |
//! | `_azure_tenant_id`, `_azure_authority_host` | Azure AD tenant and authority |
//! | `_token_exchange` | Exchange the token (RFC 8693): `true` for the protocol's endpoint, or a URL |
//! | `_token_exchange_subject_token_type` | The type of the exchanged token (default: ID token) |
//!
//! Well-known keys, used when no prefixed one is set: `azure_client_id`,
//! `azure_client_secret`, `azure_tenant_id`, `azure_authority_host`,
//! `azure_federated_token_file`, `azure_msi_endpoint`, `azure_object_id`,
//! `azure_msi_resource_id`, `azure_use_azure_cli` (also `azure_storage_*` spellings),
//! `google_service_account`, `google_service_account_key`,
//! `google_application_credentials`; and olai-http client keys such as `allow_http` and
//! `timeout`. With the `databricks` feature, `databricks_*` keys configure the Databricks
//! credential chain.

use std::collections::HashMap;
use std::fmt;

use olai_http::bearer::{
    AzureManagedIdentity, BearerAuth, BearerCredentialProvider, ClientCredentials,
    GcpServiceAccount, TokenExchange,
};
use olai_http::{ClientConfigKey, ClientOptions, CloudClient, RetryConfig};
use tokio::runtime::Handle;
use url::Url;

use crate::error::{Error, Result};

/// How to authenticate to a vending server.
///
/// When none is configured it is detected, in this order: a token; client credentials
/// with a token URL; client credentials with an Azure tenant; a token file or variable;
/// Databricks credentials (feature `databricks`); the deployment's Azure or Google
/// identity, once it can be scoped to the server (an OAuth scope or audience is set);
/// unauthenticated, if allowed. A prefixed key wins over its well-known counterpart,
/// and a client id and secret are taken as a pair, never one from each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AuthType {
    /// A static bearer token (`bearer`, `pat`).
    Bearer,
    /// OAuth 2.0 client credentials (`oauth-client-credentials`).
    OAuthClientCredentials,
    /// An Azure AD service principal (`azure-client-secret`).
    AzureClientSecret,
    /// The host's Azure managed identity (`azure-managed-identity`, `azure-msi`).
    AzureManagedIdentity,
    /// Azure workload identity (`azure-workload-identity`).
    AzureWorkloadIdentity,
    /// The signed-in Azure CLI (`azure-cli`).
    AzureCli,
    /// A Google ID token for the audience (`gcp-id-token`).
    GcpIdToken,
    /// A token read from a file (`token-file`).
    TokenFile,
    /// A token read from an environment variable (`token-env`).
    TokenEnv,
    /// The Databricks credential chain (`databricks`).
    #[cfg(feature = "databricks")]
    Databricks,
    /// No authentication (`none`).
    None,
}

impl std::str::FromStr for AuthType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "bearer" | "pat" => Self::Bearer,
            "oauth-client-credentials" => Self::OAuthClientCredentials,
            "azure-client-secret" => Self::AzureClientSecret,
            "azure-managed-identity" | "azure-msi" => Self::AzureManagedIdentity,
            "azure-workload-identity" => Self::AzureWorkloadIdentity,
            "azure-cli" => Self::AzureCli,
            "gcp-id-token" => Self::GcpIdToken,
            "token-file" => Self::TokenFile,
            "token-env" => Self::TokenEnv,
            #[cfg(feature = "databricks")]
            "databricks" => Self::Databricks,
            "none" => Self::None,
            other => return Err(Error::Config(format!("unknown auth type `{other}`"))),
        })
    }
}

/// A key of the prefixed group; see the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AuthKey {
    AuthType,
    AllowUnauthenticated,
    Token,
    TokenFile,
    TokenEnv,
    ClientId,
    ClientSecret,
    OAuthTokenUrl,
    OAuthScope,
    OAuthAudience,
    AzureTenantId,
    AzureAuthorityHost,
    TokenExchange,
    TokenExchangeSubjectTokenType,
}

impl AuthKey {
    fn parse(suffix: &str) -> Option<Self> {
        Some(match suffix {
            "auth_type" => Self::AuthType,
            "allow_unauthenticated" => Self::AllowUnauthenticated,
            "token" | "access_token" => Self::Token,
            "token_file" => Self::TokenFile,
            "token_env" => Self::TokenEnv,
            "client_id" => Self::ClientId,
            "client_secret" => Self::ClientSecret,
            "oauth_token_url" => Self::OAuthTokenUrl,
            "oauth_scope" => Self::OAuthScope,
            "oauth_audience" => Self::OAuthAudience,
            "azure_tenant_id" => Self::AzureTenantId,
            "azure_authority_host" => Self::AzureAuthorityHost,
            "token_exchange" => Self::TokenExchange,
            "token_exchange_subject_token_type" => Self::TokenExchangeSubjectTokenType,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::AuthType => "auth_type",
            Self::AllowUnauthenticated => "allow_unauthenticated",
            Self::Token => "token",
            Self::TokenFile => "token_file",
            Self::TokenEnv => "token_env",
            Self::ClientId => "client_id",
            Self::ClientSecret => "client_secret",
            Self::OAuthTokenUrl => "oauth_token_url",
            Self::OAuthScope => "oauth_scope",
            Self::OAuthAudience => "oauth_audience",
            Self::AzureTenantId => "azure_tenant_id",
            Self::AzureAuthorityHost => "azure_authority_host",
            Self::TokenExchange => "token_exchange",
            Self::TokenExchangeSubjectTokenType => "token_exchange_subject_token_type",
        }
    }

    /// Whether values of this key are secret and must not be printed.
    fn is_secret(self) -> bool {
        matches!(self, Self::Token | Self::ClientSecret)
    }
}

/// A well-known Azure identity key, named as object_store's Azure builder names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AzureKey {
    ClientId,
    ClientSecret,
    TenantId,
    AuthorityHost,
    FederatedTokenFile,
    MsiEndpoint,
    ObjectId,
    MsiResourceId,
    UseAzureCli,
}

impl AzureKey {
    fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "azure_client_id" | "azure_storage_client_id" => Self::ClientId,
            "azure_client_secret" | "azure_storage_client_secret" => Self::ClientSecret,
            "azure_tenant_id"
            | "azure_authority_id"
            | "azure_storage_tenant_id"
            | "azure_storage_authority_id" => Self::TenantId,
            "azure_authority_host" | "azure_storage_authority_host" => Self::AuthorityHost,
            "azure_federated_token_file" => Self::FederatedTokenFile,
            "azure_msi_endpoint" | "azure_identity_endpoint" => Self::MsiEndpoint,
            "azure_object_id" => Self::ObjectId,
            "azure_msi_resource_id" => Self::MsiResourceId,
            "azure_use_azure_cli" => Self::UseAzureCli,
            _ => return None,
        })
    }
}

/// A well-known Google identity key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GoogleKey {
    ServiceAccount,
    ServiceAccountKey,
    ApplicationCredentials,
}

impl GoogleKey {
    fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "google_service_account" | "google_service_account_path" => Self::ServiceAccount,
            "google_service_account_key" => Self::ServiceAccountKey,
            "google_application_credentials" => Self::ApplicationCredentials,
            _ => return None,
        })
    }
}

/// What a protocol knows about authenticating to its servers.
#[derive(Debug, Clone, Default)]
pub struct AuthDefaults {
    /// The token-exchange endpoint `_token_exchange=true` selects.
    pub token_exchange_endpoint: Option<String>,
    /// The Azure AD scope to request when none is configured, for servers whose scope
    /// is well known.
    pub oauth_scope: Option<String>,
    /// Whether the server is known to be a Databricks workspace, so the Databricks
    /// environment may be consulted.
    pub databricks: bool,
}

/// Authentication configuration for one vending protocol; see the [module docs](self).
#[derive(Clone)]
pub struct AuthConfig {
    prefix: &'static str,
    auth: HashMap<AuthKey, String>,
    azure: HashMap<AzureKey, String>,
    google: HashMap<GoogleKey, String>,
    client: Vec<(ClientConfigKey, String)>,
    #[cfg(feature = "databricks")]
    databricks: Vec<(olai_http::databricks::DatabricksConfigKey, String)>,
    from_env: bool,
    client_options: ClientOptions,
    retry: RetryConfig,
    io_runtime: Option<Handle>,
}

impl AuthConfig {
    /// An empty configuration whose prefixed keys start with `{prefix}_`.
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            auth: HashMap::new(),
            azure: HashMap::new(),
            google: HashMap::new(),
            client: Vec::new(),
            #[cfg(feature = "databricks")]
            databricks: Vec::new(),
            from_env: false,
            client_options: ClientOptions::default(),
            retry: RetryConfig::default(),
            io_runtime: None,
        }
    }

    /// The configuration read from the environment: variables named after a key
    /// (`UNITY_TOKEN` for `unity_token`, `AZURE_CLIENT_ID`, `GOOGLE_APPLICATION_CREDENTIALS`,
    /// `IDENTITY_ENDPOINT`), and, with the `databricks` feature, the Databricks chain's
    /// own variables and profile, for Databricks servers.
    pub fn from_env(prefix: &'static str) -> Self {
        let mut config = Self::new(prefix);
        config.from_env = true;
        let upper = format!("{}_", prefix.to_ascii_uppercase());
        for (name, value) in std::env::vars() {
            if name == "IDENTITY_ENDPOINT" {
                config.set("azure_msi_endpoint", value);
            } else if name.starts_with(&upper)
                || name.starts_with("AZURE_")
                || name.starts_with("GOOGLE_")
            {
                config.set(&name.to_ascii_lowercase(), value);
            }
        }
        config
    }

    /// The prefix of this configuration's keys.
    pub fn prefix(&self) -> &'static str {
        self.prefix
    }

    /// Sets `key` if it is an authentication key; returns whether it was.
    ///
    /// Lenient on purpose, so a whole storage-options map can be offered.
    pub fn set(&mut self, key: &str, value: impl Into<String>) -> bool {
        let value = value.into();
        if let Some(suffix) = key
            .strip_prefix(self.prefix)
            .and_then(|rest| rest.strip_prefix('_'))
            && let Some(key) = AuthKey::parse(suffix)
        {
            self.auth.insert(key, value);
            return true;
        }
        if let Some(key) = AzureKey::parse(key) {
            self.azure.insert(key, value);
            return true;
        }
        if let Some(key) = GoogleKey::parse(key) {
            self.google.insert(key, value);
            return true;
        }
        #[cfg(feature = "databricks")]
        if key.starts_with("databricks_")
            && let Ok(key) = key.parse::<olai_http::databricks::DatabricksConfigKey>()
        {
            self.databricks.retain(|(existing, _)| *existing != key);
            self.databricks.push((key, value));
            return true;
        }
        if let Ok(key) = key.parse::<ClientConfigKey>() {
            self.client.retain(|(existing, _)| *existing != key);
            self.client.push((key, value));
            return true;
        }
        false
    }

    /// HTTP client options for requests to the server and its identity providers.
    pub fn with_client_options(mut self, options: ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// Retries for requests to the server and its identity providers.
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Spawn the HTTP I/O on `handle`.
    pub fn with_io_runtime(mut self, handle: Handle) -> Self {
        self.io_runtime = Some(handle);
        self
    }

    /// Whether Databricks credentials are configured explicitly.
    pub fn has_databricks(&self) -> bool {
        #[cfg(feature = "databricks")]
        {
            self.databricks
                .iter()
                .any(|(key, _)| *key != olai_http::databricks::DatabricksConfigKey::Host)
        }
        #[cfg(not(feature = "databricks"))]
        false
    }

    /// The configured Databricks host, if any.
    pub fn databricks_host(&self) -> Option<String> {
        #[cfg(feature = "databricks")]
        {
            use olai_http::databricks::DatabricksConfigKey;
            self.databricks
                .iter()
                .find(|(key, _)| *key == DatabricksConfigKey::Host)
                .map(|(_, value)| value.clone())
                .or_else(|| {
                    self.from_env
                        .then(|| std::env::var("DATABRICKS_HOST").ok())
                        .flatten()
                })
        }
        #[cfg(not(feature = "databricks"))]
        None
    }

    fn get(&self, key: AuthKey) -> Option<String> {
        self.auth.get(&key).cloned()
    }

    fn key(&self, key: AuthKey) -> String {
        format!("{}_{}", self.prefix, key.name())
    }

    fn allow_unauthenticated(&self) -> bool {
        self.get(AuthKey::AllowUnauthenticated)
            .is_some_and(|value| is_truthy(&value))
    }

    /// HTTP client options: the base options, then client keys, allowing plain `http`
    /// when the server itself is plain `http`.
    pub fn http_options(&self, url: &Url) -> ClientOptions {
        let mut options = self.client_options.clone();
        for (key, value) in &self.client {
            options = options.with_config(*key, value);
        }
        let allow_http_set = self
            .client
            .iter()
            .any(|(key, _)| *key == ClientConfigKey::AllowHttp);
        if url.scheme() == "http" && !allow_http_set {
            options = options.with_allow_http(true);
        }
        options
    }

    /// The authenticated client for the server at `url`.
    pub fn client(&self, url: &Url, defaults: &AuthDefaults) -> Result<CloudClient> {
        let auth_type = match self.get(AuthKey::AuthType) {
            Some(auth_type) => auth_type.parse()?,
            None => match self.detect(url, defaults) {
                Some(auth_type) => auth_type,
                None if self.allow_unauthenticated() => AuthType::None,
                None => {
                    let p = self.prefix;
                    return Err(Error::Config(format!(
                        "no credentials configured: set `{p}_token`, `{p}_client_id` + \
                         `{p}_client_secret` (with `{p}_oauth_token_url` or \
                         `{p}_azure_tenant_id`), `{p}_token_file`, `{p}_auth_type`, an Azure \
                         or Google identity with `{p}_oauth_scope` or `{p}_oauth_audience`, \
                         or `{p}_allow_unauthenticated`"
                    )));
                }
            },
        };
        let exchange = self.token_exchange_endpoint(defaults)?;
        let options = self.http_options(url);
        match auth_type {
            AuthType::None if exchange.is_some() => Err(self.exchange_without_token()),
            AuthType::None => {
                let client = CloudClient::new_unauthenticated_with_options(
                    &options,
                    self.retry.clone(),
                    self.io_runtime.as_ref(),
                )?;
                Ok(client)
            }
            #[cfg(feature = "databricks")]
            AuthType::Databricks if exchange.is_some() => Err(self.exchange_without_token()),
            #[cfg(feature = "databricks")]
            AuthType::Databricks => self.databricks_client(url),
            _ => {
                let auth = self.bearer_auth(options.clone());
                let mut provider = self.bearer_provider(&auth, auth_type, url, defaults)?;
                if let Some(endpoint) = exchange {
                    provider = auth.token_exchange(TokenExchange {
                        endpoint,
                        subject: provider,
                        subject_token_type: self.get(AuthKey::TokenExchangeSubjectTokenType),
                        requested_token_type: None,
                        audience: None,
                        scope: None,
                    })?;
                }
                Ok(CloudClient::new_with_bearer(
                    provider,
                    &options,
                    self.retry.clone(),
                    self.io_runtime.as_ref(),
                )?)
            }
        }
    }

    fn exchange_without_token(&self) -> Error {
        Error::Config(format!(
            "`{}` needs a token to exchange: configure a token, token file or identity flow",
            self.key(AuthKey::TokenExchange)
        ))
    }

    /// The authentication type the configuration implies; see [`AuthType`].
    fn detect(&self, url: &Url, defaults: &AuthDefaults) -> Option<AuthType> {
        let has = |key| self.auth.contains_key(&key);
        let has_client = has(AuthKey::ClientId) && has(AuthKey::ClientSecret);
        if has(AuthKey::Token) {
            Some(AuthType::Bearer)
        } else if has_client && has(AuthKey::OAuthTokenUrl) {
            Some(AuthType::OAuthClientCredentials)
        } else if has_client && self.tenant_id().is_some() {
            Some(AuthType::AzureClientSecret)
        } else if has(AuthKey::TokenFile) {
            Some(AuthType::TokenFile)
        } else if has(AuthKey::TokenEnv) {
            Some(AuthType::TokenEnv)
        } else if self.databricks_configured(url, defaults) {
            #[cfg(feature = "databricks")]
            return Some(AuthType::Databricks);
            #[cfg(not(feature = "databricks"))]
            unreachable!("Databricks is never configured without the `databricks` feature")
        } else {
            self.detect_cloud_identity(defaults)
        }
    }

    /// The deployment's Azure or Google identity, once it can be scoped to the server.
    fn detect_cloud_identity(&self, defaults: &AuthDefaults) -> Option<AuthType> {
        if self.oauth_scope(defaults).is_some() {
            let client_id = self.client_id().is_some();
            let tenant_id = self.tenant_id().is_some();
            if client_id && tenant_id && self.client_secret().is_some() {
                return Some(AuthType::AzureClientSecret);
            }
            if client_id && tenant_id && self.azure.contains_key(&AzureKey::FederatedTokenFile) {
                return Some(AuthType::AzureWorkloadIdentity);
            }
            if self
                .azure
                .get(&AzureKey::UseAzureCli)
                .is_some_and(|value| is_truthy(value))
            {
                return Some(AuthType::AzureCli);
            }
            let user_assigned = [
                AzureKey::ObjectId,
                AzureKey::MsiResourceId,
                AzureKey::MsiEndpoint,
            ]
            .iter()
            .any(|key| self.azure.contains_key(key));
            if client_id || user_assigned {
                return Some(AuthType::AzureManagedIdentity);
            }
        }
        if self.auth.contains_key(&AuthKey::OAuthAudience) && !self.google.is_empty() {
            return Some(AuthType::GcpIdToken);
        }
        None
    }

    #[cfg_attr(not(feature = "databricks"), allow(unused_variables))]
    fn databricks_configured(&self, url: &Url, defaults: &AuthDefaults) -> bool {
        #[cfg(feature = "databricks")]
        {
            let target = defaults.databricks
                || self.databricks_host().is_some()
                || (self.from_env
                    && std::env::vars().any(|(name, _)| name.starts_with("DATABRICKS_")));
            self.has_databricks()
                || (self.from_env && target && self.databricks_client(url).is_ok())
        }
        #[cfg(not(feature = "databricks"))]
        false
    }

    /// The client id: prefixed, else the Azure one, unless only a prefixed secret is
    /// set, which must not pair with another identity's id.
    fn client_id(&self) -> Option<String> {
        self.get(AuthKey::ClientId).or_else(|| {
            (!self.auth.contains_key(&AuthKey::ClientSecret))
                .then(|| self.azure.get(&AzureKey::ClientId).cloned())
                .flatten()
        })
    }

    /// The client secret, paired like [`client_id`](Self::client_id).
    fn client_secret(&self) -> Option<String> {
        self.get(AuthKey::ClientSecret).or_else(|| {
            (!self.auth.contains_key(&AuthKey::ClientId))
                .then(|| self.azure.get(&AzureKey::ClientSecret).cloned())
                .flatten()
        })
    }

    fn tenant_id(&self) -> Option<String> {
        self.get(AuthKey::AzureTenantId)
            .or_else(|| self.azure.get(&AzureKey::TenantId).cloned())
    }

    fn authority_host(&self) -> Option<String> {
        self.get(AuthKey::AzureAuthorityHost)
            .or_else(|| self.azure.get(&AzureKey::AuthorityHost).cloned())
    }

    fn oauth_scope(&self, defaults: &AuthDefaults) -> Option<String> {
        self.get(AuthKey::OAuthScope)
            .or_else(|| defaults.oauth_scope.clone())
    }

    fn token_exchange_endpoint(&self, defaults: &AuthDefaults) -> Result<Option<String>> {
        let Some(value) = self.get(AuthKey::TokenExchange) else {
            return Ok(None);
        };
        if value.contains("://") {
            return Ok(Some(value));
        }
        if !is_truthy(&value) {
            return Ok(None);
        }
        defaults
            .token_exchange_endpoint
            .clone()
            .map(Some)
            .ok_or_else(|| {
                Error::Config(format!(
                    "this protocol has no default token-exchange endpoint: set `{}` to a URL",
                    self.key(AuthKey::TokenExchange)
                ))
            })
    }

    fn bearer_auth(&self, options: ClientOptions) -> BearerAuth {
        let auth = BearerAuth::new()
            .with_client_options(options)
            .with_retry(self.retry.clone());
        match &self.io_runtime {
            Some(handle) => auth.with_runtime(handle.clone()),
            None => auth,
        }
    }

    fn bearer_provider(
        &self,
        auth: &BearerAuth,
        auth_type: AuthType,
        url: &Url,
        defaults: &AuthDefaults,
    ) -> Result<BearerCredentialProvider> {
        let required = |value: Option<String>, keys: &[String]| {
            value.ok_or_else(|| {
                let keys = keys
                    .iter()
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(" or ");
                Error::Config(format!("auth type `{auth_type:?}` requires {keys}"))
            })
        };
        let key = |key| self.key(key);
        let client_id_keys = [key(AuthKey::ClientId), "azure_client_id".into()];
        let client_secret_keys = [key(AuthKey::ClientSecret), "azure_client_secret".into()];
        let tenant_keys = [key(AuthKey::AzureTenantId), "azure_tenant_id".into()];
        let scope = || required(self.oauth_scope(defaults), &[key(AuthKey::OAuthScope)]);
        let azure = |key| self.azure.get(&key).cloned();

        Ok(match auth_type {
            AuthType::Bearer => BearerAuth::static_token(required(
                self.get(AuthKey::Token),
                &[key(AuthKey::Token)],
            )?),
            AuthType::OAuthClientCredentials => auth.client_credentials(ClientCredentials {
                token_url: required(
                    self.get(AuthKey::OAuthTokenUrl),
                    &[key(AuthKey::OAuthTokenUrl)],
                )?,
                client_id: required(self.get(AuthKey::ClientId), &[key(AuthKey::ClientId)])?,
                client_secret: required(
                    self.get(AuthKey::ClientSecret),
                    &[key(AuthKey::ClientSecret)],
                )?,
                scope: self.get(AuthKey::OAuthScope),
                audience: self.get(AuthKey::OAuthAudience),
            })?,
            AuthType::AzureClientSecret => auth.azure_client_secret(
                &required(self.tenant_id(), &tenant_keys)?,
                required(self.client_id(), &client_id_keys)?,
                required(self.client_secret(), &client_secret_keys)?,
                &scope()?,
                self.authority_host(),
            )?,
            AuthType::AzureManagedIdentity => {
                let scope = scope()?;
                let resource = scope.strip_suffix("/.default").unwrap_or(&scope);
                auth.azure_managed_identity(AzureManagedIdentity {
                    resource: resource.to_owned(),
                    client_id: self.client_id(),
                    object_id: azure(AzureKey::ObjectId),
                    msi_resource_id: azure(AzureKey::MsiResourceId),
                    endpoint: azure(AzureKey::MsiEndpoint),
                })?
            }
            AuthType::AzureWorkloadIdentity => auth.azure_workload_identity(
                &required(self.tenant_id(), &tenant_keys)?,
                required(self.client_id(), &client_id_keys)?,
                required(
                    self.get(AuthKey::TokenFile)
                        .or_else(|| azure(AzureKey::FederatedTokenFile)),
                    &[key(AuthKey::TokenFile), "azure_federated_token_file".into()],
                )?,
                &scope()?,
                self.authority_host(),
            )?,
            AuthType::AzureCli => auth.azure_cli(scope()?),
            AuthType::GcpIdToken => {
                let audience = self
                    .get(AuthKey::OAuthAudience)
                    .unwrap_or_else(|| url.origin().ascii_serialization());
                let google = |key| self.google.get(&key).cloned();
                let account = google(GoogleKey::ServiceAccountKey)
                    .map(GcpServiceAccount::Key)
                    .or_else(|| {
                        google(GoogleKey::ServiceAccount)
                            .or_else(|| google(GoogleKey::ApplicationCredentials))
                            .map(GcpServiceAccount::File)
                    });
                match account {
                    Some(account) => auth.gcp_service_account_id_token(account, audience)?,
                    None => auth.gcp_id_token(audience, None)?,
                }
            }
            AuthType::TokenFile => auth.token_file(required(
                self.get(AuthKey::TokenFile),
                &[key(AuthKey::TokenFile)],
            )?),
            AuthType::TokenEnv => auth.token_env(required(
                self.get(AuthKey::TokenEnv),
                &[key(AuthKey::TokenEnv)],
            )?),
            #[cfg(feature = "databricks")]
            AuthType::Databricks => unreachable!("not a bearer flow"),
            AuthType::None => unreachable!("not a bearer flow"),
        })
    }

    #[cfg(feature = "databricks")]
    fn databricks_client(&self, url: &Url) -> Result<CloudClient> {
        use olai_http::databricks::{DatabricksBuilder, DatabricksConfigKey};

        let mut builder = if self.from_env {
            DatabricksBuilder::from_env()
        } else {
            DatabricksBuilder::new()
        };
        builder = builder
            .with_client_options(self.http_options(url))
            .with_retry(self.retry.clone());
        for (key, value) in &self.databricks {
            builder = builder.with_config(*key, value);
        }
        if !self
            .databricks
            .iter()
            .any(|(key, _)| *key == DatabricksConfigKey::Host)
        {
            // OAuth flows resolve their token endpoint against the workspace host, the
            // origin of the server URL.
            builder = builder.with_config(
                DatabricksConfigKey::Host,
                url.origin().ascii_serialization(),
            );
        }
        Ok(CloudClient::new_databricks_from_builder(
            builder,
            self.io_runtime.as_ref(),
        )?)
    }
}

impl fmt::Debug for AuthConfig {
    // Secret values (tokens, client secrets) are never printed, and only the names of
    // well-known and client keys are.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let auth: Vec<(&str, Option<&str>)> = self
            .auth
            .iter()
            .map(|(key, value)| (key.name(), (!key.is_secret()).then_some(value.as_str())))
            .collect();
        let mut f = f.debug_struct("AuthConfig");
        f.field("prefix", &self.prefix)
            .field("auth", &auth)
            .field("azure", &self.azure.keys().collect::<Vec<_>>())
            .field("google", &self.google.keys().collect::<Vec<_>>())
            .field(
                "client",
                &self
                    .client
                    .iter()
                    .map(|(key, _)| key.as_ref())
                    .collect::<Vec<_>>(),
            );
        #[cfg(feature = "databricks")]
        f.field(
            "databricks",
            &self
                .databricks
                .iter()
                .map(|(key, _)| key.as_ref())
                .collect::<Vec<_>>(),
        );
        f.field("from_env", &self.from_env)
            .field("io_runtime", &self.io_runtime.is_some())
            .finish()
    }
}

impl From<olai_http::Error> for Error {
    fn from(err: olai_http::Error) -> Self {
        Error::vend(err)
    }
}

/// Whether `value` reads as true (`1`, `true`, `on`, `yes`, `y`).
pub(crate) fn is_truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes" | "y"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> AuthConfig {
        let mut config = AuthConfig::new("unity");
        for (key, value) in pairs {
            assert!(config.set(key, *value), "{key} is an auth key");
        }
        config
    }

    fn url() -> Url {
        "https://uc.example.com/api/2.1/unity-catalog/"
            .parse()
            .unwrap()
    }

    fn detected(pairs: &[(&str, &str)], defaults: &AuthDefaults) -> Option<AuthType> {
        config(pairs).detect(&url(), defaults)
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let mut config = AuthConfig::new("unity");
        assert!(!config.set("aws_region", "us-east-1"));
        // Another protocol's prefix is not this one's.
        assert!(!config.set("iceberg_token", "t"));
        // Unprefixed Databricks spellings would capture shared keys like `token`.
        assert!(!config.set("token", "t"));
    }

    #[test]
    fn detection_order() {
        let none = AuthDefaults::default();
        assert_eq!(
            detected(&[("unity_token", "t")], &none),
            Some(AuthType::Bearer)
        );
        assert_eq!(
            detected(
                &[
                    ("unity_client_id", "id"),
                    ("unity_client_secret", "s"),
                    ("unity_oauth_token_url", "https://idp/token"),
                ],
                &none
            ),
            Some(AuthType::OAuthClientCredentials)
        );
        assert_eq!(
            detected(
                &[
                    ("unity_client_id", "id"),
                    ("unity_client_secret", "s"),
                    ("azure_tenant_id", "tenant"),
                ],
                &none
            ),
            Some(AuthType::AzureClientSecret)
        );
        assert_eq!(
            detected(&[("unity_token_file", "/var/run/token")], &none),
            Some(AuthType::TokenFile)
        );
        assert_eq!(detected(&[], &none), None);
    }

    #[test]
    fn cloud_identity_needs_a_scope() {
        let identity = [
            ("azure_client_id", "id"),
            ("azure_msi_endpoint", "http://msi"),
        ];
        assert_eq!(detected(&identity, &AuthDefaults::default()), None);
        assert_eq!(
            detected(
                &[
                    identity.as_slice(),
                    &[("unity_oauth_scope", "api://uc/.default")]
                ]
                .concat(),
                &AuthDefaults::default()
            ),
            Some(AuthType::AzureManagedIdentity)
        );
        let known_scope = AuthDefaults {
            oauth_scope: Some("api://uc/.default".into()),
            ..AuthDefaults::default()
        };
        assert_eq!(
            detected(&identity, &known_scope),
            Some(AuthType::AzureManagedIdentity)
        );
        assert_eq!(
            detected(
                &[
                    ("google_application_credentials", "/sa.json"),
                    ("unity_oauth_audience", "https://uc"),
                ],
                &AuthDefaults::default()
            ),
            Some(AuthType::GcpIdToken)
        );
    }

    #[test]
    fn client_id_and_secret_pair_up() {
        // A prefixed secret never pairs with the well-known id.
        let mixed = config(&[("unity_client_secret", "s"), ("azure_client_id", "other")]);
        assert_eq!(mixed.client_id(), None);
        let azure = config(&[("azure_client_id", "id"), ("azure_client_secret", "s")]);
        assert_eq!(azure.client_id().as_deref(), Some("id"));
        assert_eq!(azure.client_secret().as_deref(), Some("s"));
    }

    #[test]
    fn missing_credentials_name_the_keys() {
        let err = config(&[])
            .client(&url(), &AuthDefaults::default())
            .err()
            .expect("no credentials");
        assert!(err.to_string().contains("`unity_token`"), "{err}");
        assert!(
            config(&[("unity_allow_unauthenticated", "true")])
                .client(&url(), &AuthDefaults::default())
                .is_ok()
        );
    }

    #[test]
    fn token_exchange_endpoint() {
        let defaults = AuthDefaults {
            token_exchange_endpoint: Some("https://uc.example.com/exchange".into()),
            ..AuthDefaults::default()
        };
        let exchange = |value: &str| {
            config(&[("unity_token_exchange", value)]).token_exchange_endpoint(&defaults)
        };
        assert_eq!(
            exchange("true").unwrap().as_deref(),
            Some("https://uc.example.com/exchange")
        );
        assert_eq!(
            exchange("https://idp/x").unwrap().as_deref(),
            Some("https://idp/x")
        );
        assert_eq!(exchange("false").unwrap(), None);
        let err = config(&[
            ("unity_token_exchange", "true"),
            ("unity_allow_unauthenticated", "1"),
        ])
        .client(&url(), &defaults)
        .err()
        .expect("nothing to exchange");
        assert!(err.to_string().contains("needs a token"), "{err}");
    }

    #[test]
    fn plain_http_servers_allow_http() {
        let options = config(&[]).http_options(&"http://localhost:8080/api".parse().unwrap());
        assert_eq!(
            options
                .get_config_value(&ClientConfigKey::AllowHttp)
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn debug_hides_secrets() {
        let config = config(&[
            ("unity_token", "SECRET-TOKEN"),
            ("unity_client_secret", "SECRET-CLIENT"),
            ("azure_client_secret", "SECRET-AZURE"),
            ("unity_client_id", "visible-id"),
        ]);
        let debug = format!("{config:?}");
        for secret in ["SECRET-TOKEN", "SECRET-CLIENT", "SECRET-AZURE"] {
            assert!(!debug.contains(secret), "{debug}");
        }
        assert!(debug.contains("visible-id"), "{debug}");
    }
}
