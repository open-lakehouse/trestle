//! Configuring a [`UnityCatalog`] from string keys.

use std::fmt;

use olai_http::CloudClient;
use olai_storage_credentials::AccessLevel;
use url::Url;

use super::{UnityCatalog, UnityClient};
use crate::auth::{AuthConfig, AuthDefaults};
use crate::error::{Error, Result};
use crate::store::StoreOptions;

/// The UC REST API below a server root.
const API_PATH: &str = "api/2.1/unity-catalog/";

/// The OSS Unity Catalog token-exchange endpoint below a server root.
const TOKEN_EXCHANGE_PATH: &str = "api/1.0/unity-control/auth/tokens";

/// Builds a [`UnityCatalog`] from configuration keys.
///
/// | Key | Meaning |
/// |---|---|
/// | `unity_catalog_url`, `unity_url` | The REST API base, e.g. `https://host/api/2.1/unity-catalog/`; derived from the Databricks host when unset (feature `databricks`) |
/// | `unity_operation` | `read` or `read_write` for every store. Unset: ask for read-write and fall back to read when it is denied |
/// | `aws_region`, `unity_aws_region` | The region of vended S3 locations, overriding hints and discovery |
/// | `aws_endpoint`, `aws_endpoint_url` | The S3 endpoint, for servers backed by an S3-compatible store |
/// | `unity_*` auth keys, `azure_*`, `google_*`, client keys | Authentication; see [`crate::auth`] |
///
/// Client keys (`allow_http`, `timeout`, ...) apply to the stores too.
///
/// ```no_run
/// # fn run() -> olai_vended_store::Result<()> {
/// use olai_vended_store::unity::UnityBuilder;
///
/// // An OSS Unity Catalog behind an identity provider: exchange the pod's projected
/// // service-account token for a Unity Catalog token.
/// let catalog = UnityBuilder::new()
///     .with_url("https://uc.example.com/api/2.1/unity-catalog/")
///     .with_options([
///         ("unity_token_file", "/var/run/secrets/tokens/uc-token"),
///         ("unity_token_exchange", "true"),
///     ])
///     .build()?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct UnityBuilder {
    url: Option<String>,
    operation: Option<String>,
    aws_region: Option<String>,
    aws_endpoint: Option<String>,
    #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
    store_client: Vec<(object_store::ClientConfigKey, String)>,
    auth: AuthConfig,
    store_options: StoreOptions,
    client: Option<CloudClient>,
}

impl Default for UnityBuilder {
    fn default() -> Self {
        Self::with_auth(AuthConfig::new("unity"))
    }
}

impl UnityBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder configured from the environment: `UNITY_*` variables named after a
    /// key (`UNITY_CATALOG_URL`, `UNITY_TOKEN`, ...), `AWS_REGION`, `AWS_ENDPOINT`, and
    /// the identity variables [`AuthConfig::from_env`] reads. Options set afterwards win.
    pub fn from_env() -> Self {
        let mut builder = Self::with_auth(AuthConfig::from_env("unity"));
        for (name, value) in std::env::vars() {
            if name.starts_with("UNITY_") || matches!(name.as_str(), "AWS_REGION" | "AWS_ENDPOINT")
            {
                builder = builder.with_option(&name.to_ascii_lowercase(), value);
            }
        }
        builder
    }

    fn with_auth(auth: AuthConfig) -> Self {
        Self {
            url: None,
            operation: None,
            aws_region: None,
            aws_endpoint: None,
            #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
            store_client: Vec::new(),
            auth,
            store_options: StoreOptions::default(),
            client: None,
        }
    }

    /// Sets the REST API base URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Asks for `access` for every store, without fallback.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.operation = Some(
            match access {
                AccessLevel::Read => "read",
                AccessLevel::ReadWrite => "read_write",
            }
            .to_owned(),
        );
        self
    }

    /// Store options; the region, endpoint and client keys set here apply on top.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.store_options = options;
        self
    }

    /// Sends every Unity Catalog request through `client`, bypassing the
    /// authentication keys: for custom signing or a decorated transport.
    pub fn with_client(mut self, client: CloudClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Sets one option; unknown keys are ignored.
    pub fn with_option(mut self, key: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        match key {
            "unity_catalog_url" | "unity_url" => self.url = Some(value),
            "unity_operation" => self.operation = Some(value),
            "aws_region" | "unity_aws_region" => self.aws_region = Some(value),
            "aws_endpoint" | "aws_endpoint_url" => self.aws_endpoint = Some(value),
            _ => {
                #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
                if let Ok(client_key) = key.parse::<object_store::ClientConfigKey>() {
                    self.store_client
                        .retain(|(existing, _)| *existing != client_key);
                    self.store_client.push((client_key, value.clone()));
                }
                self.auth.set(key, value);
            }
        }
        self
    }

    /// Sets options from a map, ignoring keys that are not Unity Catalog, auth or client
    /// keys, so a whole storage-options map can be passed.
    pub fn with_options<I, K, V>(self, options: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        options.into_iter().fold(self, |builder, (key, value)| {
            builder.with_option(key.as_ref(), value)
        })
    }

    /// Builds the catalog. No request is sent until a store is asked for.
    pub fn build(self) -> Result<UnityCatalog> {
        let url = self.resolve_url()?;
        let access = match self.operation.as_deref() {
            None => None,
            Some("read") => Some(AccessLevel::Read),
            Some("read_write") => Some(AccessLevel::ReadWrite),
            Some(other) => {
                return Err(Error::Config(format!(
                    "`unity_operation` must be `read` or `read_write`, got `{other}`"
                )));
            }
        };
        let client = match &self.client {
            Some(client) => client.clone(),
            None => self.auth.client(&url, &self.auth_defaults(&url))?,
        };

        #[cfg_attr(
            not(any(feature = "aws", feature = "azure", feature = "gcp")),
            allow(unused_mut)
        )]
        let mut options = self.store_options.clone();
        #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
        if !self.store_client.is_empty() {
            let mut client_options = object_store::ClientOptions::new();
            for (key, value) in &self.store_client {
                client_options = client_options.with_config(*key, value);
            }
            options = options.with_client_options(client_options);
        }
        #[cfg(feature = "aws")]
        {
            if let Some(region) = &self.aws_region {
                options = options.with_aws_region(region);
            }
            if let Some(endpoint) = &self.aws_endpoint {
                options = options.with_s3_endpoint(endpoint);
            }
        }
        Ok(UnityCatalog::new(UnityClient::new(client, url))
            .with_store_options(options)
            .with_default_access(access))
    }

    /// The REST API base: explicit, else below the Databricks host.
    fn resolve_url(&self) -> Result<Url> {
        let url = match &self.url {
            Some(url) => url.clone(),
            None => {
                let host = self
                    .auth
                    .databricks_host()
                    .ok_or_else(|| Error::Config("missing `unity_catalog_url`".to_owned()))?;
                let host = if host.contains("://") {
                    host
                } else {
                    format!("https://{host}")
                };
                format!("{}/{API_PATH}", host.trim_end_matches('/'))
            }
        };
        let mut url = Url::parse(&url)
            .map_err(|e| Error::Config(format!("invalid `unity_catalog_url` `{url}`: {e}")))?;
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(url)
    }

    fn auth_defaults(&self, url: &Url) -> AuthDefaults {
        // The token-exchange endpoint sits beside the REST API, below the server root.
        let root = url
            .as_str()
            .strip_suffix(API_PATH)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}/", url.origin().ascii_serialization()));
        AuthDefaults {
            token_exchange_endpoint: Some(format!("{root}{TOKEN_EXCHANGE_PATH}")),
            #[cfg(feature = "databricks")]
            oauth_scope: databricks::host_in(url, databricks::AZURE_DOMAINS)
                .then(|| databricks::AZURE_SCOPE.to_owned()),
            #[cfg(not(feature = "databricks"))]
            oauth_scope: None,
            #[cfg(feature = "databricks")]
            databricks: databricks::host_in(url, databricks::DOMAINS)
                || databricks::host_in(url, databricks::AZURE_DOMAINS),
            #[cfg(not(feature = "databricks"))]
            databricks: false,
        }
    }
}

impl fmt::Debug for UnityBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnityBuilder")
            .field("url", &self.url)
            .field("operation", &self.operation)
            .field("aws_region", &self.aws_region)
            .field("aws_endpoint", &self.aws_endpoint)
            .field("auth", &self.auth)
            .field("client", &self.client.as_ref().map(|_| "CloudClient"))
            .finish_non_exhaustive()
    }
}

/// Databricks workspaces, which serve Unity Catalog.
#[cfg(feature = "databricks")]
mod databricks {
    use url::Url;

    /// The Azure AD scope of Azure Databricks.
    pub(super) const AZURE_SCOPE: &str = "2ff814a6-3304-4ab8-85cb-cd0e6f879c1d/.default";

    pub(super) const AZURE_DOMAINS: &[&str] = &[
        "azuredatabricks.net",
        "databricks.azure.cn",
        "databricks.azure.us",
    ];

    pub(super) const DOMAINS: &[&str] = &[
        "cloud.databricks.com",
        "cloud.databricks.us",
        "cloud.databricks.mil",
        "gcp.databricks.com",
    ];

    /// Whether the host of `url` is one of `domains` or below one.
    pub(super) fn host_in(url: &Url, domains: &[&str]) -> bool {
        url.host_str().is_some_and(|host| {
            domains.iter().any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
        })
    }
}
