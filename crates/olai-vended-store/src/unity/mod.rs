//! Unity Catalog REST (feature `unity`, native only).
//!
//! Vends credentials through the `temporary-{table,volume,path}-credentials` endpoints and
//! builds stores from them. The client is hand-written over olai-http and covers only
//! what vending needs: one lookup to turn a name into an id and storage location, and
//! the vend calls.
//!
//! ```no_run
//! # async fn run() -> olai_vended_store::Result<()> {
//! use object_store::ObjectStore;
//! use olai_vended_store::AccessLevel;
//! use olai_vended_store::unity::{UnityCatalog, UnityClient};
//!
//! let http = olai_http::CloudClient::new_with_token("<token>");
//! let base = "https://uc.example.com/api/2.1/unity-catalog/".parse().unwrap();
//! let catalog = UnityCatalog::new(UnityClient::new(http, base));
//!
//! let volume = catalog
//!     .store_for_url("uc:///Volumes/main/default/landing/raw", AccessLevel::Read)
//!     .await?;
//! let listing = volume.prefixed().list(None);
//! # Ok(()) }
//! ```

mod client;
mod config;
#[cfg(feature = "delta-api")]
#[cfg_attr(docsrs, doc(cfg(feature = "delta-api")))]
pub mod delta_api;
mod reference;
mod source;

use std::sync::Arc;

use olai_storage_credentials::AccessLevel;
use url::Url;

pub use self::client::{
    ApiError, PathOperation, TableInfo, TableOperation, UnityClient, VolumeInfo, VolumeOperation,
};
pub use self::config::UnityBuilder;
pub use self::reference::UnityReference;
pub use self::source::{Securable, UnitySource};
use crate::error::{Error, Result};
use crate::source::CredentialSource;
use crate::store::{ScopedStore, StoreOptions, build_store};

/// Builds stores for Unity Catalog securables.
///
/// Each store is bound to the securable and access level it was built for: a refresh
/// replays exactly that vend, so it never widens. Cheap to clone.
#[derive(Debug, Clone)]
pub struct UnityCatalog {
    client: UnityClient,
    options: StoreOptions,
    default_access: Option<AccessLevel>,
    #[cfg(feature = "delta-api")]
    delta_api: bool,
}

impl UnityCatalog {
    /// Builds stores through `client` with default [`StoreOptions`].
    pub fn new(client: UnityClient) -> Self {
        Self {
            client,
            options: StoreOptions::default(),
            default_access: None,
            #[cfg(feature = "delta-api")]
            delta_api: false,
        }
    }

    /// Vend table credentials through the Delta API instead of
    /// `temporary-table-credentials`, as catalog-managed tables need. Volumes and paths
    /// are unaffected.
    #[cfg(feature = "delta-api")]
    pub fn with_delta_api(mut self, delta_api: bool) -> Self {
        self.delta_api = delta_api;
        self
    }

    /// A Delta API source for the table `catalog.schema.table`.
    #[cfg(feature = "delta-api")]
    pub async fn delta_table_source(
        &self,
        catalog: &str,
        schema: &str,
        table: &str,
        access: AccessLevel,
    ) -> Result<delta_api::DeltaApiSource> {
        delta_api::DeltaApiSource::load(self.client.clone(), catalog, schema, table, access).await
    }

    /// A builder configured from string keys; see [`UnityBuilder`].
    pub fn builder() -> UnityBuilder {
        UnityBuilder::new()
    }

    /// The access [`open`](Self::open) asks for. `None` (the default) asks for
    /// read-write and falls back to read when that is denied.
    pub fn with_default_access(mut self, access: Option<AccessLevel>) -> Self {
        self.default_access = access;
        self
    }

    /// Configures the stores this builds.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.options = options;
        self
    }

    /// The Unity Catalog client.
    pub fn client(&self) -> &UnityClient {
        &self.client
    }

    /// A source for the table `full_name` (`catalog.schema.table`).
    pub async fn table_source(&self, full_name: &str, access: AccessLevel) -> Result<UnitySource> {
        let table = self.client.get_table(full_name).await?;
        let location = table.storage_location.ok_or_else(|| {
            Error::InvalidReference(format!("table `{full_name}` has no storage location"))
        })?;
        UnitySource::new(
            self.client.clone(),
            Securable::Table(table.table_id),
            &location,
            access,
        )
    }

    /// A source for the volume `full_name` (`catalog.schema.volume`).
    pub async fn volume_source(&self, full_name: &str, access: AccessLevel) -> Result<UnitySource> {
        let volume = self.client.get_volume(full_name).await?;
        let location = volume.storage_location.ok_or_else(|| {
            Error::InvalidReference(format!("volume `{full_name}` has no storage location"))
        })?;
        UnitySource::new(
            self.client.clone(),
            Securable::Volume(volume.volume_id),
            &location,
            access,
        )
    }

    /// A source for a storage URL governed by an external location.
    pub fn path_source(&self, url: &Url, access: AccessLevel) -> Result<UnitySource> {
        UnitySource::new(
            self.client.clone(),
            Securable::Path(url.clone()),
            url.as_str(),
            access,
        )
    }

    /// Builds the store for `reference`. A volume reference with a sub-path yields a
    /// store whose prefix is that sub-path.
    pub async fn store(
        &self,
        reference: &UnityReference,
        access: AccessLevel,
    ) -> Result<ScopedStore> {
        let (source, sub_path) = self.resolve(reference, access).await?;
        let seed = source.fetch().await?;
        self.build(seed, source, sub_path).await
    }

    /// Builds the store for `url` at the [default access](Self::with_default_access).
    ///
    /// Without one, it asks for read-write, and asks again for read only if the server
    /// denies that (HTTP 403 or `PERMISSION_DENIED`); any other failure is returned.
    /// The name is looked up once either way.
    pub async fn open(&self, url: &str) -> Result<ScopedStore> {
        let reference = UnityReference::parse(url)?;
        if let Some(access) = self.default_access {
            return self.store(&reference, access).await;
        }
        let (source, sub_path) = self.resolve(&reference, AccessLevel::ReadWrite).await?;
        match source.fetch().await {
            Ok(seed) => self.build(seed, source, sub_path).await,
            Err(err) if ApiError::find(&err).is_some_and(ApiError::is_permission_denied) => {
                let source = source.with_access(AccessLevel::Read);
                let seed = source.fetch().await?;
                self.build(seed, source, sub_path).await
            }
            Err(err) => Err(err),
        }
    }

    async fn build(
        &self,
        seed: olai_storage_credentials::VendedCredential,
        source: Resolved,
        sub_path: &str,
    ) -> Result<ScopedStore> {
        let store = build_store(&seed, Arc::new(source), &self.options).await?;
        store.child(sub_path)
    }

    /// The source for `reference`, and the sub-path below its storage root.
    async fn resolve<'r>(
        &self,
        reference: &'r UnityReference,
        access: AccessLevel,
    ) -> Result<(Resolved, &'r str)> {
        let full_name = reference.full_name();
        Ok(match reference {
            #[cfg(feature = "delta-api")]
            UnityReference::Table {
                catalog,
                schema,
                table,
            } if self.delta_api => (
                Resolved::DeltaApi(
                    self.delta_table_source(catalog, schema, table, access)
                        .await?,
                ),
                "",
            ),
            UnityReference::Table { .. } => (
                Resolved::Unity(
                    self.table_source(full_name.as_deref().unwrap_or_default(), access)
                        .await?,
                ),
                "",
            ),
            UnityReference::Volume { path, .. } => (
                Resolved::Unity(
                    self.volume_source(full_name.as_deref().unwrap_or_default(), access)
                        .await?,
                ),
                path.as_str(),
            ),
            UnityReference::Path(url) => (Resolved::Unity(self.path_source(url, access)?), ""),
        })
    }

    /// Builds the store for a URL [`UnityReference`] parses: `uc:///Tables/...`,
    /// `uc:///Volumes/...`, the legacy `uc://catalog.schema.table`, or a storage URL.
    pub async fn store_for_url(&self, url: &str, access: AccessLevel) -> Result<ScopedStore> {
        self.store(&UnityReference::parse(url)?, access).await
    }
}

/// The source a reference resolved to.
#[derive(Debug)]
enum Resolved {
    Unity(UnitySource),
    #[cfg(feature = "delta-api")]
    DeltaApi(delta_api::DeltaApiSource),
}

impl Resolved {
    fn with_access(self, access: AccessLevel) -> Self {
        match self {
            Self::Unity(source) => Self::Unity(source.with_access(access)),
            #[cfg(feature = "delta-api")]
            Self::DeltaApi(source) => Self::DeltaApi(source.with_access(access)),
        }
    }
}

#[async_trait::async_trait]
impl CredentialSource for Resolved {
    async fn fetch(&self) -> Result<olai_storage_credentials::VendedCredential> {
        match self {
            Self::Unity(source) => source.fetch().await,
            #[cfg(feature = "delta-api")]
            Self::DeltaApi(source) => source.fetch().await,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "aws")]
    use mockito::Matcher;
    use mockito::{Server, ServerGuard};
    #[cfg(feature = "aws")]
    use object_store::path::Path;
    use olai_http::CloudClient;
    use serde_json::json;

    use super::*;

    fn catalog(server: &ServerGuard) -> UnityCatalog {
        let base = format!("{}/api/2.1/unity-catalog", server.url())
            .parse()
            .unwrap();
        let options = StoreOptions::new();
        #[cfg(feature = "aws")]
        let options = options.without_region_discovery();
        UnityCatalog::new(UnityClient::new(
            CloudClient::new_with_token("uc-token"),
            base,
        ))
        .with_store_options(options)
    }

    #[cfg(feature = "aws")]
    fn aws_credentials(url: &str) -> String {
        json!({
            "aws_temp_credentials": {
                "access_key_id": "AKID",
                "secret_access_key": "secret",
                "session_token": "session",
            },
            "expiration_time": 4_102_444_800_000_i64,
            "url": url,
        })
        .to_string()
    }

    #[cfg(any(feature = "aws", feature = "fs"))]
    async fn mock_table(server: &mut ServerGuard, name: &str, id: &str, location: &str) {
        server
            .mock(
                "GET",
                format!("/api/2.1/unity-catalog/tables/{name}").as_str(),
            )
            .match_header("authorization", "Bearer uc-token")
            .with_body(json!({ "table_id": id, "storage_location": location }).to_string())
            .create_async()
            .await;
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn table_store_vends_once_and_refreshes_the_same_request() {
        let mut server = Server::new_async().await;
        mock_table(&mut server, "main.default.t", "tid", "s3://bucket/tables/t").await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .match_body(Matcher::Json(
                json!({ "table_id": "tid", "operation": "READ" }),
            ))
            .with_body(aws_credentials("s3://bucket/tables/t"))
            .expect(2)
            .create_async()
            .await;
        let catalog = catalog(&server);

        let store = catalog
            .store_for_url("uc:///Tables/main/default/t", AccessLevel::Read)
            .await
            .unwrap();
        assert_eq!(store.url().as_str(), "s3://bucket/tables/t");
        assert_eq!(store.prefix().as_ref(), "tables/t");

        // A refresh replays the bound request, never a wider one.
        let source = catalog
            .table_source("main.default.t", AccessLevel::Read)
            .await
            .unwrap();
        let vended = source.fetch().await.unwrap();
        assert_eq!(vended.access, AccessLevel::Read);
        vend.assert_async().await;
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn legacy_table_url_and_read_write() {
        let mut server = Server::new_async().await;
        mock_table(&mut server, "main.default.t", "tid", "s3://bucket/t").await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .match_body(Matcher::Json(
                json!({ "table_id": "tid", "operation": "READ_WRITE" }),
            ))
            .with_body(aws_credentials("s3://bucket/t"))
            .create_async()
            .await;
        catalog(&server)
            .store_for_url("uc://main.default.t", AccessLevel::ReadWrite)
            .await
            .unwrap();
        vend.assert_async().await;
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn volume_sub_path_narrows_the_store() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/api/2.1/unity-catalog/volumes/main.default.landing")
            .with_body(
                json!({ "volume_id": "vid", "storage_location": "s3://bucket/vols/landing" })
                    .to_string(),
            )
            .create_async()
            .await;
        server
            .mock(
                "POST",
                "/api/2.1/unity-catalog/temporary-volume-credentials",
            )
            .match_body(Matcher::Json(
                json!({ "volume_id": "vid", "operation": "READ_VOLUME" }),
            ))
            .with_body(aws_credentials("s3://bucket/vols/landing"))
            .create_async()
            .await;
        let store = catalog(&server)
            .store_for_url(
                "uc:///Volumes/main/default/landing/raw/2026",
                AccessLevel::Read,
            )
            .await
            .unwrap();
        assert_eq!(store.prefix(), &Path::from("vols/landing/raw/2026"));
        assert_eq!(store.url().as_str(), "s3://bucket/vols/landing/raw/2026");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn path_credentials_for_a_storage_url() {
        let mut server = Server::new_async().await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-path-credentials")
            .match_body(Matcher::Json(
                json!({ "url": "s3://bucket/ext/data", "operation": "PATH_READ_WRITE" }),
            ))
            .with_body(aws_credentials("s3://bucket/ext/data"))
            .create_async()
            .await;
        let store = catalog(&server)
            .store_for_url("s3://bucket/ext/data", AccessLevel::ReadWrite)
            .await
            .unwrap();
        assert_eq!(store.prefix().as_ref(), "ext/data");
        vend.assert_async().await;
    }

    #[cfg(feature = "fs")]
    #[tokio::test]
    async fn local_tables_are_not_vended() {
        let mut server = Server::new_async().await;
        mock_table(
            &mut server,
            "main.default.local",
            "tid",
            "file:///tmp/uc/local",
        )
        .await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .expect(0)
            .create_async()
            .await;
        let store = catalog(&server)
            .store_for_url("uc:///Tables/main/default/local", AccessLevel::Read)
            .await
            .unwrap();
        assert_eq!(store.prefix().as_ref(), "tmp/uc/local");
        vend.assert_async().await;
    }

    #[tokio::test]
    async fn api_errors_carry_status_and_code() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/api/2.1/unity-catalog/tables/main.default.secret")
            .with_status(403)
            .with_body(
                json!({ "error_code": "PERMISSION_DENIED", "message": "no USE SCHEMA" })
                    .to_string(),
            )
            .create_async()
            .await;
        let err = catalog(&server)
            .table_source("main.default.secret", AccessLevel::Read)
            .await
            .unwrap_err();
        let api = ApiError::find(&err).expect("an ApiError");
        assert!(api.is_permission_denied(), "{api}");
        assert_eq!(api.message, "no USE SCHEMA");
        assert!(err.to_string().contains("403 PERMISSION_DENIED"), "{err}");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn credential_for_unrelated_storage_is_rejected() {
        let mut server = Server::new_async().await;
        mock_table(&mut server, "main.default.t", "tid", "s3://bucket/t").await;
        server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .with_body(aws_credentials("s3://other-bucket/t"))
            .create_async()
            .await;
        let err = catalog(&server)
            .store_for_url("uc:///Tables/main/default/t", AccessLevel::Read)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Credentials(_)), "{err}");
    }

    fn denied() -> String {
        json!({ "error_code": "PERMISSION_DENIED", "message": "no MODIFY" }).to_string()
    }

    fn builder(server: &ServerGuard) -> UnityBuilder {
        UnityBuilder::new().with_options([
            (
                "unity_catalog_url",
                format!("{}/api/2.1/unity-catalog", server.url()),
            ),
            ("unity_token", "uc-token".to_owned()),
        ])
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn open_falls_back_to_read_when_write_is_denied() {
        let mut server = Server::new_async().await;
        let lookup = server
            .mock("GET", "/api/2.1/unity-catalog/tables/main.default.t")
            .with_body(
                json!({ "table_id": "tid", "storage_location": "s3://bucket/t" }).to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let write = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .match_body(Matcher::Json(
                json!({ "table_id": "tid", "operation": "READ_WRITE" }),
            ))
            .with_status(403)
            .with_body(denied())
            .create_async()
            .await;
        let read = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .match_body(Matcher::Json(
                json!({ "table_id": "tid", "operation": "READ" }),
            ))
            .with_body(aws_credentials("s3://bucket/t"))
            .create_async()
            .await;
        let store = builder(&server)
            .with_store_options(StoreOptions::new().without_region_discovery())
            .build()
            .unwrap()
            .open("uc:///Tables/main/default/t")
            .await
            .unwrap();
        assert_eq!(store.prefix().as_ref(), "t");
        lookup.assert_async().await;
        write.assert_async().await;
        read.assert_async().await;
    }

    #[tokio::test]
    async fn open_does_not_fall_back_on_other_failures() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/api/2.1/unity-catalog/tables/main.default.t")
            .with_body(
                json!({ "table_id": "tid", "storage_location": "s3://bucket/t" }).to_string(),
            )
            .create_async()
            .await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .with_status(400)
            .with_body(json!({ "error_code": "INVALID_PARAMETER_VALUE" }).to_string())
            .expect(1)
            .create_async()
            .await;
        let err = builder(&server)
            .build()
            .unwrap()
            .open("uc:///Tables/main/default/t")
            .await
            .unwrap_err();
        assert_eq!(
            ApiError::find(&err).and_then(|e| e.status),
            Some(400),
            "{err}"
        );
        vend.assert_async().await;
    }

    #[tokio::test]
    async fn explicit_operation_never_falls_back() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/api/2.1/unity-catalog/tables/main.default.t")
            .with_body(
                json!({ "table_id": "tid", "storage_location": "s3://bucket/t" }).to_string(),
            )
            .create_async()
            .await;
        let vend = server
            .mock("POST", "/api/2.1/unity-catalog/temporary-table-credentials")
            .with_status(403)
            .with_body(denied())
            .expect(1)
            .create_async()
            .await;
        let err = builder(&server)
            .with_option("unity_operation", "read_write")
            .build()
            .unwrap()
            .open("uc:///Tables/main/default/t")
            .await
            .unwrap_err();
        assert!(
            ApiError::find(&err).is_some_and(ApiError::is_permission_denied),
            "{err}"
        );
        vend.assert_async().await;
    }

    #[test]
    fn builder_validates_its_keys() {
        let err = UnityBuilder::new()
            .with_option("unity_token", "t")
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("unity_catalog_url"), "{err}");
        let err = UnityBuilder::new()
            .with_options([("unity_url", "https://uc/api"), ("unity_token", "t")])
            .with_option("unity_operation", "write")
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("unity_operation"), "{err}");
        let err = UnityBuilder::new()
            .with_option("unity_url", "https://uc/api")
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("no credentials"), "{err}");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn parse_url_opts_dispatches_to_unity() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/api/2.1/unity-catalog/volumes/main.default.landing")
            .match_header("authorization", "Bearer uc-token")
            .with_body(
                json!({ "volume_id": "vid", "storage_location": "s3://bucket/vols/landing" })
                    .to_string(),
            )
            .create_async()
            .await;
        server
            .mock(
                "POST",
                "/api/2.1/unity-catalog/temporary-volume-credentials",
            )
            .match_body(Matcher::Json(
                json!({ "volume_id": "vid", "operation": "READ_VOLUME" }),
            ))
            .with_body(aws_credentials("s3://bucket/vols/landing"))
            .create_async()
            .await;
        let url = "uc:///Volumes/main/default/landing/raw".parse().unwrap();
        let (store, path) = crate::parse_url_opts(
            &url,
            [
                (
                    "unity_catalog_url",
                    format!("{}/api/2.1/unity-catalog", server.url()),
                ),
                ("unity_token", "uc-token".to_owned()),
                ("unity_operation", "read".to_owned()),
                ("aws_region", "us-east-1".to_owned()),
                // Keys for other stores are ignored.
                ("azure_storage_account_name", "acct".to_owned()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(path, Path::from("vols/landing/raw"));
        assert_eq!(store.to_string(), "AmazonS3(bucket)");
    }

    #[tokio::test]
    async fn names_cannot_add_path_segments() {
        let mut server = Server::new_async().await;
        let lookup = server
            .mock("GET", "/api/2.1/unity-catalog/tables/main.default.a%2Fb")
            .with_status(404)
            .with_body("{}")
            .create_async()
            .await;
        let _ = catalog(&server)
            .table_source("main.default.a/b", AccessLevel::Read)
            .await;
        lookup.assert_async().await;
    }
}
