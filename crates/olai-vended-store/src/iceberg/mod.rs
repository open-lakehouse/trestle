//! Iceberg REST (feature `iceberg`, native only).
//!
//! Loads tables with `X-Iceberg-Access-Delegation: vended-credentials` and builds
//! stores from the `storage-credentials` the catalog returns (or, from older servers,
//! the credential keys in the table config). Stores refresh through the table's
//! `credentials` endpoint, falling back to `loadTable` on servers without it. The
//! client is hand-written over olai-http and covers only what vending needs: the
//! catalog config, `loadTable`, and the credentials endpoint.
//!
//! Iceberg has no standard table-reference URL, so catalogs are configured through
//! [`IcebergBuilder`] (or [`IcebergCatalog::new`]) and tables are named by namespace
//! and name; [`parse_url_opts`](crate::parse_url_opts) does not dispatch to them.
//!
//! ```no_run
//! # async fn run() -> olai_vended_store::Result<()> {
//! use object_store::ObjectStore;
//! use olai_vended_store::iceberg::IcebergBuilder;
//!
//! let catalog = IcebergBuilder::new()
//!     .with_options([
//!         ("iceberg_uri", "https://catalog.example.com/catalog/"),
//!         ("iceberg_warehouse", "analytics"),
//!         ("iceberg_token", "<token>"),
//!     ])
//!     .connect()
//!     .await?;
//! let table = catalog.table_store(&["sales"], "orders").await?;
//! let listing = table.prefixed().list(None);
//! # Ok(()) }
//! ```

mod client;
mod config;
mod source;

use std::sync::Arc;

use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::{AccessLevel, StorageScope};

pub use self::client::{CatalogConfig, IcebergClient, LoadTable};
pub use self::config::IcebergBuilder;
pub use self::source::IcebergSource;
use crate::api::{local_credential, storage_root};
use crate::error::{Error, Result};
use crate::store::{ScopedStore, StoreOptions, build_store};

/// Builds stores for Iceberg REST tables.
///
/// Each store is bound to its table and storage prefix: a refresh replays the vend for
/// that table and keeps the credential for that prefix, so it never widens. Cheap to
/// clone.
#[derive(Debug, Clone)]
pub struct IcebergCatalog {
    client: IcebergClient,
    options: StoreOptions,
    access: AccessLevel,
}

impl IcebergCatalog {
    /// Builds stores through `client` with default [`StoreOptions`], declared
    /// read-only.
    pub fn new(client: IcebergClient) -> Self {
        Self {
            client,
            options: StoreOptions::default(),
            access: AccessLevel::Read,
        }
    }

    /// A builder configured from string keys; see [`IcebergBuilder`].
    pub fn builder() -> IcebergBuilder {
        IcebergBuilder::new()
    }

    /// Declares the access stores are built for (read by default).
    ///
    /// Iceberg REST cannot ask for an access level; the catalog grants what the
    /// principal may do. The declaration labels the credentials, so a read-only store
    /// refuses to presign writes.
    pub fn with_access(mut self, access: AccessLevel) -> Self {
        self.access = access;
        self
    }

    /// Configures the stores this builds.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.options = options;
        self
    }

    /// The Iceberg REST client.
    pub fn client(&self) -> &IcebergClient {
        &self.client
    }

    /// A source for the whole of `namespace.table`. Loads the table for its location.
    pub async fn table_source(&self, namespace: &[&str], table: &str) -> Result<IcebergSource> {
        let (source, _) = self.load(namespace, table).await?;
        Ok(source)
    }

    /// Builds the store for the whole of `namespace.table`, from the most specific
    /// credential covering its location.
    pub async fn table_store(&self, namespace: &[&str], table: &str) -> Result<ScopedStore> {
        let (source, loaded) = self.load(namespace, table).await?;
        self.build(&loaded, source).await
    }

    /// Builds one store for each prefix of `namespace.table` the catalog vended a
    /// credential for, most specific first: for tables whose data and metadata are
    /// covered by different credentials. Register them in a
    /// [`RoutingObjectStore`](crate::RoutingObjectStore) to route each path to its
    /// credential.
    ///
    /// Credentials for prefixes broader than the table are clamped to it, and
    /// credentials for unrelated storage are ignored.
    pub async fn table_stores(&self, namespace: &[&str], table: &str) -> Result<Vec<ScopedStore>> {
        let (table_source, loaded) = self.load(namespace, table).await?;
        let ctx = VendContext::new(table_source.scope().clone(), self.access);
        let mut scopes: Vec<StorageScope> = if loaded.credentials.storage_credentials.is_empty() {
            vec![table_source.scope().clone()]
        } else {
            loaded
                .credentials
                .to_vended(&ctx)?
                .into_iter()
                .map(|vended| vended.scope)
                .collect()
        };
        scopes.sort_by_cached_key(|scope| (std::cmp::Reverse(scope.depth()), scope.to_string()));
        scopes.dedup();

        let mut stores = Vec::with_capacity(scopes.len());
        for scope in scopes {
            let source = IcebergSource::new(
                self.client.clone(),
                owned(namespace),
                table.to_owned(),
                scope,
                table_source.hints().clone(),
                self.access,
            );
            stores.push(self.build(&loaded, source).await?);
        }
        Ok(stores)
    }

    /// Loads `namespace.table` and binds a source to its location.
    async fn load(&self, namespace: &[&str], table: &str) -> Result<(IcebergSource, LoadTable)> {
        let namespace = owned(namespace);
        let loaded = self.client.load_table(&namespace, table).await?;
        let location = loaded.location().ok_or_else(|| {
            Error::InvalidReference(format!(
                "Iceberg REST returned no location for table `{}.{table}`",
                namespace.join(".")
            ))
        })?;
        let (scope, hints) = storage_root("Iceberg REST", &location)?;
        let source = IcebergSource::new(
            self.client.clone(),
            namespace,
            table.to_owned(),
            scope,
            hints,
            self.access,
        );
        Ok((source, loaded))
    }

    /// Builds the store for `source`, seeded from the `loadTable` response it was
    /// loaded with, so building does not vend again.
    async fn build(&self, loaded: &LoadTable, source: IcebergSource) -> Result<ScopedStore> {
        self.options
            .check_server_storage("Iceberg REST", source.scope())?;
        let ctx = VendContext::new(source.scope().clone(), source.access());
        let seed = match local_credential(source.scope(), source.access()) {
            Some(local) => local,
            None => {
                let mut seed = source::vend_from_table(loaded, &ctx)?;
                seed.hints.fill_from(source.hints());
                seed
            }
        };
        build_store(&seed, Arc::new(source), &self.options).await
    }
}

fn owned(namespace: &[&str]) -> Vec<String> {
    namespace.iter().map(|level| (*level).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server, ServerGuard};
    use olai_http::CloudClient;
    use serde_json::json;

    use super::*;
    use crate::ApiError;
    use crate::source::CredentialSource;

    /// Recorded `loadTable` and `credentials` responses, shared with the wire tests.
    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                "../../../olai-storage-credentials/tests/fixtures/iceberg/",
                $name
            ))
        };
    }

    const TABLE: &str = "/v1/wh/namespaces/ns/tables/tab";

    fn catalog(server: &ServerGuard) -> IcebergCatalog {
        let uri = server.url().parse().unwrap();
        let options = StoreOptions::new();
        #[cfg(feature = "aws")]
        let options = options.without_region_discovery();
        IcebergCatalog::new(
            IcebergClient::new(CloudClient::new_with_token("t"), uri).with_prefix("wh"),
        )
        .with_store_options(options)
    }

    async fn load_table(server: &mut ServerGuard, body: &str) -> mockito::Mock {
        server
            .mock("GET", TABLE)
            .match_header("x-iceberg-access-delegation", "vended-credentials")
            .with_body(body)
            .create_async()
            .await
    }

    fn key_id(vended: &olai_storage_credentials::VendedCredential) -> &str {
        match &vended.credentials {
            olai_storage_credentials::StorageCredentials::Aws { key_id, .. } => key_id,
            other => panic!("expected AWS credentials, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_tables_are_refused_by_default() {
        let mut server = Server::new_async().await;
        let body = json!({
            "metadata-location": "file:///etc/tab/metadata/00001.metadata.json",
            "metadata": { "location": "file:///etc/tab" },
        })
        .to_string();
        load_table(&mut server, &body).await;
        let err = catalog(&server)
            .table_store(&["ns"], "tab")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        #[cfg(feature = "fs")]
        catalog(&server)
            .with_store_options(StoreOptions::new().with_local_storage(true))
            .table_store(&["ns"], "tab")
            .await
            .unwrap();
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn connect_applies_the_config_prefix() {
        let mut server = Server::new_async().await;
        let config = server
            .mock("GET", "/v1/config")
            .match_query(Matcher::UrlEncoded("warehouse".into(), "analytics".into()))
            .with_body(json!({ "defaults": { "prefix": "wh" }, "overrides": {} }).to_string())
            .expect(1)
            .create_async()
            .await;
        let load = load_table(&mut server, fixture!("load_table_s3.json")).await;
        let catalog = IcebergBuilder::new()
            .with_options([
                ("iceberg_uri", server.url()),
                ("iceberg_warehouse", "analytics".to_owned()),
                ("iceberg_token", "t".to_owned()),
            ])
            .with_store_options(StoreOptions::new().without_region_discovery())
            .connect()
            .await
            .unwrap();
        let store = catalog.table_store(&["ns"], "tab").await.unwrap();
        assert_eq!(store.prefix().as_ref(), "warehouse/ns/tab");
        assert_eq!(store.url().as_str(), "s3://bucket/warehouse/ns/tab");
        config.assert_async().await;
        load.assert_async().await;
    }

    #[tokio::test]
    async fn refresh_asks_the_credentials_endpoint() {
        let mut server = Server::new_async().await;
        load_table(&mut server, fixture!("load_table_s3.json")).await;
        let credentials = server
            .mock("GET", format!("{TABLE}/credentials").as_str())
            .with_body(fixture!("load_credentials.json"))
            .expect(2)
            .create_async()
            .await;
        let source = catalog(&server).table_source(&["ns"], "tab").await.unwrap();
        for _ in 0..2 {
            let vended = source.fetch().await.unwrap();
            // The bucket-wide credential covers the table; the data-only, sibling and
            // other-bucket ones do not. It is clamped to the table.
            assert_eq!(key_id(&vended), "ak-wide");
            assert_eq!(vended.scope.to_string(), "s3://bucket/warehouse/ns/tab");
        }
        credentials.assert_async().await;
    }

    #[tokio::test]
    async fn servers_without_the_credentials_endpoint_replay_load_table() {
        let mut server = Server::new_async().await;
        let load = server
            .mock("GET", TABLE)
            .with_body(fixture!("load_table_s3.json"))
            .expect(2)
            .create_async()
            .await;
        server
            .mock("GET", format!("{TABLE}/credentials").as_str())
            .with_status(404)
            .with_body(
                json!({ "error": { "message": "no such route", "type": "NotFound", "code": 404 } })
                    .to_string(),
            )
            .create_async()
            .await;
        let source = catalog(&server).table_source(&["ns"], "tab").await.unwrap();
        let vended = source.fetch().await.unwrap();
        assert_eq!(key_id(&vended), "ak-test");
        assert!(vended.expires_at.is_some());
        assert_eq!(vended.hints.region.as_deref(), Some("us-west-2"));
        load.assert_async().await;
    }

    #[tokio::test]
    async fn remote_signing_is_refused() {
        let mut server = Server::new_async().await;
        load_table(&mut server, fixture!("load_table_remote_signing.json")).await;
        let err = catalog(&server)
            .table_store(&["ns"], "tab")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("remotely"), "{err}");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn several_credentials_route_to_their_prefixes() {
        let mut server = Server::new_async().await;
        let body = json!({
            "metadata-location": "s3://bucket/warehouse/ns/tab/metadata/00001.metadata.json",
            "storage-credentials": [
                { "prefix": "s3://bucket/warehouse/ns/tab/",
                  "config": { "s3.access-key-id": "ak-table", "s3.secret-access-key": "s" } },
                { "prefix": "s3://bucket/warehouse/ns/tab/data/",
                  "config": { "s3.access-key-id": "ak-data", "s3.secret-access-key": "s" } },
                { "prefix": "s3://bucket/",
                  "config": { "s3.access-key-id": "ak-bucket", "s3.secret-access-key": "s" } },
                { "prefix": "s3://other/",
                  "config": { "s3.access-key-id": "ak-other", "s3.secret-access-key": "s" } },
            ]
        });
        load_table(&mut server, &body.to_string()).await;
        server
            .mock("GET", format!("{TABLE}/credentials").as_str())
            .with_body(json!({ "storage-credentials": body["storage-credentials"] }).to_string())
            .create_async()
            .await;
        let stores = catalog(&server).table_stores(&["ns"], "tab").await.unwrap();
        let prefixes: Vec<_> = stores.iter().map(|s| s.prefix().to_string()).collect();
        assert_eq!(prefixes, ["warehouse/ns/tab/data", "warehouse/ns/tab"]);

        let router = crate::RoutingObjectStore::new();
        for store in &stores {
            router.register(store).unwrap();
        }
        assert_eq!(
            router.prefixes(),
            stores
                .iter()
                .map(|s| s.prefix().clone())
                .collect::<Vec<_>>()
        );

        // Each prefix refreshes to its own, most specific credential.
        let table = catalog(&server).table_source(&["ns"], "tab").await.unwrap();
        assert_eq!(key_id(&table.fetch().await.unwrap()), "ak-table");
    }

    #[cfg(feature = "azure")]
    #[tokio::test]
    async fn adls_credentials_are_picked_by_account() {
        let mut server = Server::new_async().await;
        load_table(&mut server, fixture!("load_table_adls.json")).await;
        let store = catalog(&server).table_store(&["ns"], "tab").await.unwrap();
        assert_eq!(store.prefix().as_ref(), "warehouse/ns/tab");
    }

    #[tokio::test]
    async fn namespaces_are_one_path_segment() {
        let mut server = Server::new_async().await;
        let load = server
            .mock("GET", "/v1/wh/namespaces/a%1Fb%2Fc/tables/t")
            .with_body(fixture!("load_table_s3.json"))
            .expect(1)
            .create_async()
            .await;
        catalog(&server)
            .table_source(&["a", "b/c"], "t")
            .await
            .unwrap();
        load.assert_async().await;
    }

    #[tokio::test]
    async fn errors_carry_the_exception_type() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", TABLE)
            .with_status(404)
            .with_body(
                json!({ "error": {
                    "message": "Table does not exist: ns.tab",
                    "type": "NoSuchTableException",
                    "code": 404
                } })
                .to_string(),
            )
            .create_async()
            .await;
        let err = catalog(&server)
            .table_source(&["ns"], "tab")
            .await
            .unwrap_err();
        let api = ApiError::find(&err).expect("an ApiError");
        assert_eq!(api.code.as_deref(), Some("NoSuchTableException"));
        assert_eq!(api.message, "Table does not exist: ns.tab");
        assert!(api.is_not_found());
    }

    #[tokio::test]
    async fn client_credentials_use_the_spec_endpoint_and_scope() {
        let mut server = Server::new_async().await;
        let token = server
            .mock("POST", "/v1/oauth/tokens")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "client_credentials".into()),
                Matcher::UrlEncoded("client_id".into(), "id".into()),
                Matcher::UrlEncoded("client_secret".into(), "secret".into()),
                Matcher::UrlEncoded("scope".into(), "catalog".into()),
            ]))
            .with_body(
                json!({ "access_token": "tok", "token_type": "bearer", "expires_in": 3600 })
                    .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let config = server
            .mock("GET", "/v1/config")
            .match_header("authorization", "Bearer tok")
            .with_body(json!({ "overrides": { "prefix": "wh" } }).to_string())
            .expect(1)
            .create_async()
            .await;
        let catalog = IcebergBuilder::new()
            .with_options([
                ("iceberg_uri", server.url()),
                ("iceberg_credential", "id:secret".to_owned()),
            ])
            .connect()
            .await
            .unwrap();
        assert_eq!(
            format!("{:?}", catalog.client()),
            format!(
                "IcebergClient {{ uri: \"{}/\", prefix: \"wh\", .. }}",
                server.url()
            )
        );
        token.assert_async().await;
        config.assert_async().await;
    }
}
