//! Configuring an [`IcebergCatalog`] from string keys.

use std::fmt;

use olai_http::CloudClient;
use olai_storage_credentials::AccessLevel;
use url::Url;

use super::{IcebergCatalog, IcebergClient};
use crate::auth::{AuthConfig, AuthDefaults};
use crate::error::{Error, Result};
use crate::options::{StoreKeys, access_key_value, parse_access, parse_base_url};
use crate::store::StoreOptions;

/// The OAuth token endpoint the Iceberg REST spec places below the catalog URI.
const OAUTH_TOKENS_PATH: &str = "v1/oauth/tokens";

/// The scope Iceberg REST clients request with client credentials.
const CATALOG_SCOPE: &str = "catalog";

/// Builds an [`IcebergCatalog`] from configuration keys.
///
/// | Key | Meaning |
/// |---|---|
/// | `iceberg_uri` | The catalog URI, below which the `v1/` paths sit |
/// | `iceberg_warehouse` | The warehouse, sent to `GET v1/config` |
/// | `iceberg_prefix` | The path prefix; when set, `GET v1/config` is skipped |
/// | `iceberg_operation` | `read` (the default) or `read_write`: the access the stores are declared with. Iceberg REST cannot ask for one; the catalog grants what the principal may do |
/// | `iceberg_credential` | OAuth client credentials as `client_id:client_secret`; a bare secret is refused |
/// | `iceberg_oauth2_server_uri` | The OAuth token endpoint; default `{uri}/v1/oauth/tokens` |
/// | `aws_region` | The region of vended S3 locations, overriding hints and discovery |
/// | `aws_endpoint`, `aws_endpoint_url` | The S3 endpoint, for catalogs backed by an S3-compatible store |
/// | `allow_local_storage` | Accept `file://` tables, as a local development catalog has; see [`StoreOptions::with_local_storage`] |
/// | `iceberg_*` auth keys, `azure_*`, `google_*`, client keys | Authentication; see [`crate::auth`]. Client credentials request the scope `catalog` unless `iceberg_oauth_scope` is set |
///
/// Client keys (`allow_http`, `timeout`, ...) apply to the stores too.
///
/// ```no_run
/// # async fn run() -> olai_vended_store::Result<()> {
/// use olai_vended_store::iceberg::IcebergBuilder;
///
/// let catalog = IcebergBuilder::new()
///     .with_options([
///         ("iceberg_uri", "https://catalog.example.com/catalog/"),
///         ("iceberg_warehouse", "analytics"),
///         ("iceberg_credential", "<client id>:<client secret>"),
///     ])
///     .connect()
///     .await?;
/// let store = catalog.table_store(&["sales"], "orders").await?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct IcebergBuilder {
    uri: Option<String>,
    warehouse: Option<String>,
    prefix: Option<String>,
    operation: Option<String>,
    /// `iceberg_credential` was a bare secret, which is refused on connect.
    bare_credential: bool,
    store_keys: StoreKeys,
    auth: AuthConfig,
    store_options: StoreOptions,
    client: Option<CloudClient>,
}

impl Default for IcebergBuilder {
    fn default() -> Self {
        Self::with_auth(AuthConfig::new("iceberg"))
    }
}

impl IcebergBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder configured from the environment: `ICEBERG_*` variables named after a
    /// key (`ICEBERG_URI`, `ICEBERG_TOKEN`, ...), `AWS_REGION`, `AWS_ENDPOINT`, and the
    /// identity variables [`AuthConfig::from_env`] reads. Options set afterwards win.
    pub fn from_env() -> Self {
        let mut builder = Self::with_auth(AuthConfig::from_env("iceberg"));
        for (name, value) in std::env::vars() {
            if name.starts_with("ICEBERG_")
                || matches!(name.as_str(), "AWS_REGION" | "AWS_ENDPOINT")
            {
                builder = builder.with_option(&name.to_ascii_lowercase(), value);
            }
        }
        builder
    }

    fn with_auth(auth: AuthConfig) -> Self {
        Self {
            uri: None,
            warehouse: None,
            prefix: None,
            operation: None,
            bare_credential: false,
            store_keys: StoreKeys::default(),
            auth,
            store_options: StoreOptions::default(),
            client: None,
        }
    }

    /// Sets the catalog URI.
    pub fn with_uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    /// Sets the warehouse.
    pub fn with_warehouse(mut self, warehouse: impl Into<String>) -> Self {
        self.warehouse = Some(warehouse.into());
        self
    }

    /// Declares the access every store is built for.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.operation = Some(access_key_value(access).to_owned());
        self
    }

    /// Store options; the region, endpoint and client keys set here apply on top.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.store_options = options;
        self
    }

    /// Sends every catalog request through `client`, bypassing the authentication
    /// keys.
    pub fn with_client(mut self, client: CloudClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Sets one option; unknown keys are ignored.
    pub fn with_option(mut self, key: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        match key {
            "iceberg_uri" => self.uri = Some(value),
            "iceberg_warehouse" => self.warehouse = Some(value),
            "iceberg_prefix" => self.prefix = Some(value),
            "iceberg_operation" => self.operation = Some(value),
            "iceberg_credential" => match value.split_once(':') {
                Some((id, secret)) => {
                    self.auth.set("iceberg_client_id", id);
                    self.auth.set("iceberg_client_secret", secret);
                    self.bare_credential = false;
                }
                // Other clients accept a bare secret, with the id left to the server;
                // the client-credentials flow here always sends an id.
                None => self.bare_credential = true,
            },
            "iceberg_oauth2_server_uri" => {
                self.auth.set("iceberg_oauth_token_url", value);
            }
            _ => {
                if !self.store_keys.set(key, &value) {
                    self.auth.set(key, value);
                }
            }
        }
        self
    }

    /// Sets options from a map, ignoring keys that are not Iceberg, auth, store or
    /// client keys, so a whole storage-options map can be passed.
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

    /// Builds the catalog, fetching its config (`GET v1/config`) for the URI override
    /// and path prefix, unless `iceberg_prefix` is set.
    pub async fn connect(self) -> Result<IcebergCatalog> {
        if self.bare_credential {
            return Err(Error::Config(
                "`iceberg_credential` must be `client_id:client_secret`; a bare secret is \
                 not supported"
                    .to_owned(),
            ));
        }
        let uri = parse_base_url(
            "iceberg_uri",
            self.uri
                .as_deref()
                .ok_or_else(|| Error::Config("missing `iceberg_uri`".to_owned()))?,
        )?;
        let access = parse_access("iceberg_operation", self.operation.as_deref())?
            .unwrap_or(AccessLevel::Read);
        let http = match &self.client {
            Some(client) => client.clone(),
            None => self.auth.client(&uri, &auth_defaults(&uri))?,
        };
        let client = IcebergClient::new(http, uri);
        let client = match &self.prefix {
            Some(prefix) => client.with_prefix(prefix),
            None => {
                let config = client.config(self.warehouse.as_deref()).await?;
                client.configured(&config)?
            }
        };
        Ok(IcebergCatalog::new(client)
            .with_access(access)
            .with_store_options(self.store_keys.apply(self.store_options.clone())))
    }
}

/// The spec's OAuth defaults: tokens from `{uri}/v1/oauth/tokens`, scope `catalog`.
fn auth_defaults(uri: &Url) -> AuthDefaults {
    AuthDefaults {
        oauth_token_url: uri.join(OAUTH_TOKENS_PATH).ok().map(String::from),
        client_credentials_scope: Some(CATALOG_SCOPE.to_owned()),
        ..AuthDefaults::default()
    }
}

impl fmt::Debug for IcebergBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IcebergBuilder")
            .field("uri", &self.uri)
            .field("warehouse", &self.warehouse)
            .field("prefix", &self.prefix)
            .field("operation", &self.operation)
            .field("store_keys", &self.store_keys)
            .field("auth", &self.auth)
            .field("client", &self.client.as_ref().map(|_| "CloudClient"))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bare_credentials_are_refused() {
        let err = IcebergBuilder::new()
            .with_options([
                ("iceberg_uri", "https://catalog.example.com/"),
                ("iceberg_prefix", "wh"),
                ("iceberg_credential", "BARE-SECRET"),
            ])
            .connect()
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{err}");
        assert!(err.to_string().contains("client_id:client_secret"), "{err}");
        assert!(!err.to_string().contains("BARE-SECRET"), "{err}");
    }

    #[tokio::test]
    async fn paired_credentials_configure_client_credentials() {
        IcebergBuilder::new()
            .with_options([
                ("iceberg_uri", "https://catalog.example.com/"),
                ("iceberg_prefix", "wh"),
                ("iceberg_credential", "BARE-SECRET"),
                ("iceberg_credential", "id:secret"),
            ])
            .connect()
            .await
            .unwrap();
    }
}
