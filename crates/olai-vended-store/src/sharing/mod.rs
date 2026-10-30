//! Delta / Open Sharing (feature `sharing`, native only).
//!
//! Vends read credentials for shared volumes and agent skills through Open Sharing's
//! `temporary-volume-credentials` and `temporary-skill-credentials` endpoints, and
//! builds stores from them. The server and the recipient's token come from a sharing
//! profile. The client is hand-written over olai-http and covers only what vending
//! needs: one lookup for the asset's storage location, and the vend.
//!
//! The query protocol (table metadata and file listing) and presigned-file mode are
//! out of scope: they return URLs, not credentials.
//!
//! ```no_run
//! # async fn run() -> olai_vended_store::Result<()> {
//! use object_store::ObjectStore;
//! use olai_vended_store::sharing::OpenSharing;
//!
//! let sharing = OpenSharing::from_profile("config.share")?;
//! let volume = sharing.open("sales.raw.landing/2024").await?;
//! let listing = volume.prefixed().list(None);
//! # Ok(()) }
//! ```

mod client;
mod config;
mod profile;
mod reference;
mod source;

use std::path::Path;
use std::sync::Arc;

pub use self::client::{AssetInfo, AssetKind, SharingClient};
pub use self::config::SharingBuilder;
pub use self::profile::SharingProfile;
pub use self::reference::SharingReference;
pub use self::source::SharingSource;
use crate::error::Result;
use crate::source::CredentialSource;
use crate::store::{ScopedStore, StoreOptions, build_store};

/// Builds stores for assets shared with a recipient.
///
/// Each store is bound to its asset: a refresh replays exactly that vend. Cheap to
/// clone.
#[derive(Debug, Clone)]
pub struct OpenSharing {
    client: SharingClient,
    options: StoreOptions,
}

impl OpenSharing {
    /// Builds stores through `client` with default [`StoreOptions`].
    pub fn new(client: SharingClient) -> Self {
        Self {
            client,
            options: StoreOptions::default(),
        }
    }

    /// Builds stores for the server and token of the profile file at `path`.
    pub fn from_profile(path: impl AsRef<Path>) -> Result<Self> {
        SharingBuilder::new()
            .with_profile(SharingProfile::from_file(path)?)
            .build()
    }

    /// A builder configured from string keys; see [`SharingBuilder`].
    pub fn builder() -> SharingBuilder {
        SharingBuilder::new()
    }

    /// Configures the stores this builds.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.options = options;
        self
    }

    /// The Open Sharing client.
    pub fn client(&self) -> &SharingClient {
        &self.client
    }

    /// A source for the volume `share.schema.volume`.
    pub async fn volume_source(
        &self,
        share: &str,
        schema: &str,
        volume: &str,
    ) -> Result<SharingSource> {
        SharingSource::load(
            self.client.clone(),
            AssetKind::Volume,
            share,
            schema,
            volume,
        )
        .await
    }

    /// A source for the agent skill `share.schema.skill`.
    pub async fn skill_source(
        &self,
        share: &str,
        schema: &str,
        skill: &str,
    ) -> Result<SharingSource> {
        SharingSource::load(self.client.clone(), AssetKind::Skill, share, schema, skill).await
    }

    /// Builds the store for `source`.
    pub async fn store(&self, source: SharingSource) -> Result<ScopedStore> {
        let seed = source.fetch().await?;
        build_store(&seed, Arc::new(source), &self.options).await
    }

    /// Builds the store for a volume reference [`SharingReference`] parses:
    /// `share.schema.volume[/path]` or `sharing:///share/schema/volume[/path]`. A path
    /// yields a store whose prefix is that path.
    pub async fn open(&self, reference: &str) -> Result<ScopedStore> {
        let reference = SharingReference::parse(reference)?;
        let source = self
            .volume_source(&reference.share, &reference.schema, &reference.volume)
            .await?;
        self.store(source).await?.child(&reference.path)
    }
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server, ServerGuard};
    use serde_json::json;

    use super::*;
    use crate::ApiError;
    use crate::error::Error;

    /// Recorded `temporary-*-credentials` responses, shared with the wire tests.
    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                "../../../olai-storage-credentials/tests/fixtures/sharing/",
                $name
            ))
        };
    }

    const API: &str = "/api/1.0/opensharing";

    fn profile(server: &ServerGuard, expiration: &str) -> SharingProfile {
        SharingProfile::from_json(
            &json!({
                "shareCredentialsVersion": 1,
                "endpoint": format!("{}{API}", server.url()),
                "bearerToken": "recipient-token",
                "expirationTime": expiration,
            })
            .to_string(),
        )
        .unwrap()
    }

    fn sharing(server: &ServerGuard) -> OpenSharing {
        let options = StoreOptions::new();
        #[cfg(feature = "aws")]
        let options = options.without_region_discovery();
        SharingBuilder::new()
            .with_profile(profile(server, "2100-01-01T00:00:00Z"))
            .with_store_options(options)
            .build()
            .unwrap()
    }

    async fn volume(server: &mut ServerGuard, location: &str) -> mockito::Mock {
        server
            .mock(
                "GET",
                format!("{API}/shares/sales/schemas/raw/volumes/vol").as_str(),
            )
            .match_header("authorization", "Bearer recipient-token")
            .with_body(json!({ "name": "vol", "storage_location": location }).to_string())
            .expect(1)
            .create_async()
            .await
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn open_vends_for_the_volume_and_narrows_to_the_path() {
        let mut server = Server::new_async().await;
        let lookup = volume(&mut server, "s3://bucket/shares/vol").await;
        let vend = server
            .mock(
                "POST",
                format!("{API}/shares/sales/schemas/raw/volumes/vol/temporary-volume-credentials")
                    .as_str(),
            )
            .match_header("authorization", "Bearer recipient-token")
            .match_body(Matcher::Json(json!({})))
            .with_body(fixture!("volume_aws.json"))
            .expect(1)
            .create_async()
            .await;
        let store = sharing(&server).open("sales.raw.vol/2024").await.unwrap();
        assert_eq!(store.prefix().as_ref(), "shares/vol/2024");
        lookup.assert_async().await;
        vend.assert_async().await;
    }

    #[tokio::test]
    async fn skills_vend_through_their_endpoint() {
        let mut server = Server::new_async().await;
        server
            .mock(
                "GET",
                format!("{API}/shares/sales/schemas/raw/skills/sk").as_str(),
            )
            .with_body(
                json!({ "name": "sk", "storageLocation": "gs://bucket/shares/vol" }).to_string(),
            )
            .create_async()
            .await;
        let vend = server
            .mock(
                "POST",
                format!("{API}/shares/sales/schemas/raw/skills/sk/temporary-skill-credentials")
                    .as_str(),
            )
            .with_body(fixture!("volume_camel_case.json"))
            .expect(1)
            .create_async()
            .await;
        let source = sharing(&server)
            .skill_source("sales", "raw", "sk")
            .await
            .unwrap();
        let vended = source.fetch().await.unwrap();
        assert_eq!(
            vended.credentials,
            olai_storage_credentials::StorageCredentials::GcpBearer("ya29.test".into())
        );
        assert_eq!(vended.access, olai_storage_credentials::AccessLevel::Read);
        assert!(vended.expires_at.is_some());
        vend.assert_async().await;
    }

    #[tokio::test]
    async fn local_volumes_need_no_vend() {
        let mut server = Server::new_async().await;
        volume(&mut server, "file:///data/shares/vol").await;
        let vend = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let source = sharing(&server)
            .volume_source("sales", "raw", "vol")
            .await
            .unwrap();
        let vended = source.fetch().await.unwrap();
        assert_eq!(
            vended.credentials,
            olai_storage_credentials::StorageCredentials::None
        );
        vend.assert_async().await;
    }

    #[test]
    fn expired_profiles_are_refused() {
        let server = mockito::Server::new();
        let err = SharingBuilder::new()
            .with_profile(profile(&server, "2000-01-01T00:00:00Z"))
            .build()
            .unwrap_err();
        assert!(matches!(err, Error::Config(_)), "{err}");
        assert!(!err.to_string().contains("recipient-token"), "{err}");
    }

    #[tokio::test]
    async fn keys_override_the_profile_endpoint_and_token() {
        let mut server = Server::new_async().await;
        let lookup = server
            .mock("GET", "/other/shares/sales/schemas/raw/volumes/vol")
            .match_header("authorization", "Bearer override")
            .with_body(json!({ "name": "vol", "storage_location": "file:///v" }).to_string())
            .expect(1)
            .create_async()
            .await;
        SharingBuilder::new()
            .with_profile(profile(&server, "2100-01-01T00:00:00Z"))
            .with_options([
                ("sharing_endpoint", format!("{}/other", server.url())),
                ("sharing_token", "override".to_owned()),
            ])
            .build()
            .unwrap()
            .volume_source("sales", "raw", "vol")
            .await
            .unwrap();
        lookup.assert_async().await;
    }

    #[tokio::test]
    async fn errors_carry_the_error_code() {
        let mut server = Server::new_async().await;
        server
            .mock(
                "GET",
                format!("{API}/shares/sales/schemas/raw/volumes/vol").as_str(),
            )
            .with_status(403)
            .with_body(
                json!({ "errorCode": "PERMISSION_DENIED", "message": "not shared" }).to_string(),
            )
            .create_async()
            .await;
        let err = sharing(&server)
            .volume_source("sales", "raw", "vol")
            .await
            .unwrap_err();
        let api = ApiError::find(&err).expect("an ApiError");
        assert_eq!(api.code.as_deref(), Some("PERMISSION_DENIED"));
        assert!(api.is_permission_denied());
        assert!(
            err.to_string().starts_with("Open Sharing request failed"),
            "{err}"
        );
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn parse_url_opts_dispatches_sharing_urls() {
        let mut server = Server::new_async().await;
        volume(&mut server, "s3://bucket/shares/vol").await;
        server
            .mock(
                "POST",
                format!("{API}/shares/sales/schemas/raw/volumes/vol/temporary-volume-credentials")
                    .as_str(),
            )
            .with_body(fixture!("volume_aws.json"))
            .create_async()
            .await;
        let path = std::env::temp_dir().join(format!(
            "olai-vended-store-{}-{}.share",
            std::process::id(),
            server.socket_address().port()
        ));
        std::fs::write(
            &path,
            json!({
                "shareCredentialsVersion": 1,
                "endpoint": format!("{}{API}", server.url()),
                "bearerToken": "recipient-token",
            })
            .to_string(),
        )
        .unwrap();
        let url = "sharing:///sales/raw/vol/sub%20dir".parse().unwrap();
        let result = crate::parse_url_opts(
            &url,
            [
                ("sharing_profile", path.to_string_lossy().into_owned()),
                ("aws_region", "us-east-1".to_owned()),
            ],
        )
        .await;
        std::fs::remove_file(&path).ok();
        let (_, prefix) = result.unwrap();
        assert_eq!(prefix.as_ref(), "shares/vol/sub dir");
    }
}
