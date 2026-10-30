//! A minimal Iceberg REST client: catalog config, `loadTable` with vended credentials,
//! and the table `credentials` endpoint.

use std::collections::BTreeMap;
use std::fmt;

use olai_http::{CloudClient, CloudRequestBuilder};
use olai_storage_credentials::wire::iceberg::{LoadCredentialsResponse, LoadTableCredentials};
use serde::Deserialize;
use url::Url;

use crate::api::{ApiClient, Dialect};
use crate::error::{Error, Result};

/// The header asking a catalog to delegate storage access.
const ACCESS_DELEGATION: &str = "X-Iceberg-Access-Delegation";

/// Iceberg REST errors: `{"error": {"message": "...", "type": "...", "code": 404}}`.
const DIALECT: Dialect = Dialect {
    service: "Iceberg REST",
    envelope: |body| {
        #[derive(Deserialize)]
        struct Envelope {
            error: Model,
        }
        #[derive(Deserialize)]
        struct Model {
            message: Option<String>,
            #[serde(rename = "type")]
            kind: Option<String>,
        }
        serde_json::from_str::<Envelope>(body)
            .ok()
            .map(|e| (e.error.kind, e.error.message))
    },
};

/// A client for the Iceberg REST endpoints vending needs: `GET v1/config`,
/// `GET v1/{prefix}/namespaces/{ns}/tables/{table}` and its `credentials`.
///
/// Authentication, retries and the I/O runtime belong to the wrapped [`CloudClient`].
/// Cheap to clone.
#[derive(Clone)]
pub struct IcebergClient {
    api: ApiClient,
    prefix: Vec<String>,
}

impl IcebergClient {
    /// A client for the catalog at `uri` (the base the `v1/` paths are below), with no
    /// prefix.
    pub fn new(client: CloudClient, uri: Url) -> Self {
        Self {
            api: ApiClient::new(client, uri, DIALECT),
            prefix: Vec::new(),
        }
    }

    /// The catalog URI.
    pub fn uri(&self) -> &Url {
        self.api.base_url()
    }

    /// The same client with the path `prefix` a catalog's config names (`ice/wh`),
    /// inserted after `v1/`.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        self
    }

    /// The catalog's config for `warehouse` (`GET v1/config`).
    pub async fn config(&self, warehouse: Option<&str>) -> Result<CatalogConfig> {
        let mut request = self.api.get(&["v1", "config"])?;
        if let Some(warehouse) = warehouse {
            request = request.query(&[("warehouse", warehouse)]);
        }
        self.api.send(request).await
    }

    /// The same client with `config` applied: its `uri` override, and its `prefix`
    /// (the override, else the default).
    pub fn configured(mut self, config: &CatalogConfig) -> Result<Self> {
        if let Some(uri) = config.overrides.get("uri") {
            let uri = Url::parse(uri).map_err(|e| {
                Error::Config(format!(
                    "the catalog config overrides `uri` with `{uri}`: {e}"
                ))
            })?;
            self.api = self.api.with_base_url(uri);
        }
        Ok(match config.get("prefix") {
            Some(prefix) => self.with_prefix(prefix),
            None => self,
        })
    }

    /// Loads `namespace.table`, asking for vended credentials.
    pub async fn load_table(&self, namespace: &[String], table: &str) -> Result<LoadTable> {
        let request = self
            .table_request(namespace, table, None)?
            .header(ACCESS_DELEGATION, "vended-credentials");
        self.api.send(request).await
    }

    /// Vends fresh credentials for `namespace.table` (`GET .../credentials`).
    ///
    /// Servers predating the endpoint answer 404, 405 or 501; see
    /// [`ApiError::is_not_found`](crate::ApiError::is_not_found).
    pub async fn load_credentials(
        &self,
        namespace: &[String],
        table: &str,
    ) -> Result<LoadCredentialsResponse> {
        let request = self.table_request(namespace, table, Some("credentials"))?;
        self.api.send(request).await
    }

    /// `GET v1/{prefix}/namespaces/{ns}/tables/{table}[/{suffix}]`. The namespace
    /// levels are joined by the unit separator into one segment, so a level can never
    /// add a segment.
    fn table_request(
        &self,
        namespace: &[String],
        table: &str,
        suffix: Option<&str>,
    ) -> Result<CloudRequestBuilder> {
        let namespace = namespace.join("\u{1f}");
        let mut path = vec!["v1"];
        path.extend(self.prefix.iter().map(String::as_str));
        path.extend(["namespaces", &namespace, "tables", table]);
        path.extend(suffix);
        self.api.get(&path)
    }
}

impl fmt::Debug for IcebergClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IcebergClient")
            .field("uri", &self.uri().as_str())
            .field("prefix", &self.prefix.join("/"))
            .finish_non_exhaustive()
    }
}

/// A catalog's `GET v1/config` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CatalogConfig {
    /// Properties the client must use.
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// Properties the client uses unless configured otherwise.
    #[serde(default)]
    pub defaults: BTreeMap<String, String>,
}

impl CatalogConfig {
    /// The property `key`: the override, else the default.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.overrides
            .get(key)
            .or_else(|| self.defaults.get(key))
            .map(String::as_str)
    }
}

/// The parts of a `loadTable` response vending needs: the table location and its
/// credentials.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LoadTable {
    /// The current metadata file.
    #[serde(default)]
    pub metadata_location: Option<String>,
    #[serde(default)]
    metadata: Option<Metadata>,
    /// The vended credentials, or the legacy table config carrying them.
    #[serde(flatten)]
    pub credentials: LoadTableCredentials,
}

#[derive(Clone, Deserialize)]
struct Metadata {
    location: Option<String>,
}

impl LoadTable {
    /// The table location: `metadata.location`, else the directory above the
    /// metadata file's `metadata/` directory.
    pub fn location(&self) -> Option<String> {
        if let Some(location) = self.metadata.as_ref().and_then(|m| m.location.clone()) {
            return Some(location);
        }
        let file = self.metadata_location.as_deref()?;
        let (dir, _) = file.rsplit_once('/')?;
        Some(dir.strip_suffix("/metadata").unwrap_or(dir).to_owned())
    }
}

impl fmt::Debug for LoadTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadTable")
            .field("location", &self.location())
            .field("credentials", &self.credentials)
            .finish()
    }
}
