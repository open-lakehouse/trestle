//! Building object stores from vended credentials.

use std::fmt;
use std::sync::Arc;
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
use std::time::Duration;

use object_store::ObjectStore;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::prefix::PrefixStore;
#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
use olai_storage_credentials::StorageCredentials;
use olai_storage_credentials::{StorageLocation, StorageProvider, StorageScope, VendedCredential};
use url::Url;

use crate::error::{Error, Result};
use crate::source::CredentialSource;

/// A store for the storage one vended credential covers.
///
/// [`root`](Self::root) is rooted at the bucket (or container), so paths are full object
/// keys; this is what a router registers. The credential only authorizes paths below
/// [`prefix`](Self::prefix). [`prefixed`](Self::prefixed) is a view whose paths are
/// relative to the prefix.
#[derive(Clone)]
pub struct ScopedStore {
    root: Arc<dyn ObjectStore>,
    prefix: Path,
    scope: StorageScope,
}

impl ScopedStore {
    /// The bucket- or container-rooted store.
    ///
    /// The credential may authorize nothing outside [`prefix`](Self::prefix).
    pub fn root(&self) -> Arc<dyn ObjectStore> {
        Arc::clone(&self.root)
    }

    /// The path within [`root`](Self::root) the credential covers.
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// The storage the credential covers.
    pub fn scope(&self) -> &StorageScope {
        &self.scope
    }

    /// The canonical URL of [`scope`](Self::scope), e.g. `s3://bucket/tables/t1`.
    pub fn url(&self) -> Url {
        self.scope.location().to_url()
    }

    /// A store whose paths are relative to [`prefix`](Self::prefix).
    pub fn prefixed(&self) -> Arc<dyn ObjectStore> {
        if self.prefix.as_ref().is_empty() {
            self.root()
        } else {
            Arc::new(PrefixStore::new(self.root(), self.prefix.clone()))
        }
    }
}

impl fmt::Debug for ScopedStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedStore")
            .field("root", &self.root.to_string())
            .field("prefix", &self.prefix.as_ref())
            .field("scope", &self.scope.to_string())
            .finish()
    }
}

/// How [`build_store`] configures stores.
///
/// Cheap to clone; clones share the S3 region cache.
#[derive(Clone)]
pub struct StoreOptions {
    #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
    refresh_margin: Duration,
    #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
    clock: Option<crate::refresh::Clock>,
    #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
    client_options: object_store::ClientOptions,
    #[cfg(feature = "aws")]
    aws_region: Option<String>,
    #[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
    region_discovery: RegionDiscovery,
    #[cfg(all(
        any(feature = "aws", feature = "azure", feature = "gcp"),
        not(target_arch = "wasm32")
    ))]
    io_runtime: Option<tokio::runtime::Handle>,
}

#[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
#[derive(Clone, Debug)]
enum RegionDiscovery {
    Shared,
    Custom(crate::region::BucketRegions),
    Disabled,
}

// Not derivable once a store backend is enabled.
#[allow(clippy::derivable_impls)]
impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
            refresh_margin: crate::refresh::DEFAULT_REFRESH_MARGIN,
            #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
            clock: None,
            #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
            client_options: object_store::ClientOptions::default(),
            #[cfg(feature = "aws")]
            aws_region: None,
            #[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
            region_discovery: RegionDiscovery::Shared,
            #[cfg(all(
                any(feature = "aws", feature = "azure", feature = "gcp"),
                not(target_arch = "wasm32")
            ))]
            io_runtime: None,
        }
    }
}

impl StoreOptions {
    /// Defaults: a 5 minute refresh margin, S3 region discovery through a process-wide
    /// cache, and object_store's default HTTP client options.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
impl StoreOptions {
    /// Re-vend `margin` before expiry instead of
    /// [`DEFAULT_REFRESH_MARGIN`](crate::DEFAULT_REFRESH_MARGIN).
    pub fn with_refresh_margin(mut self, margin: Duration) -> Self {
        self.refresh_margin = margin;
        self
    }

    /// Read the time from `clock` when deciding to re-vend.
    pub fn with_clock(mut self, clock: crate::refresh::Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// HTTP client options for the stores. A credential's
    /// [`allow_http`](olai_storage_credentials::StoreHints::allow_http) hint is applied
    /// on top.
    pub fn with_client_options(mut self, options: object_store::ClientOptions) -> Self {
        self.client_options = options;
        self
    }

    /// Spawn the stores' HTTP I/O on `handle`, for callers whose own runtime must not
    /// run network I/O (for example a CPU-bound query runtime).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_io_runtime(mut self, handle: tokio::runtime::Handle) -> Self {
        self.io_runtime = Some(handle);
        self
    }

    fn provider<T: crate::FromStorageCredentials>(
        &self,
        seed: &VendedCredential,
        source: Arc<dyn CredentialSource>,
    ) -> Result<crate::RefreshingCredentialProvider<T>> {
        let mut provider = crate::RefreshingCredentialProvider::new(seed, source)?
            .with_refresh_margin(self.refresh_margin);
        if let Some(clock) = &self.clock {
            provider = provider.with_clock(Arc::clone(clock));
        }
        Ok(provider)
    }

    fn client_options(&self, seed: &VendedCredential) -> object_store::ClientOptions {
        match seed.hints.allow_http {
            Some(allow) => self.client_options.clone().with_allow_http(allow),
            None => self.client_options.clone(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn connector(&self) -> Option<object_store::client::SpawnedReqwestConnector> {
        self.io_runtime
            .clone()
            .map(object_store::client::SpawnedReqwestConnector::new)
    }
}

#[cfg(feature = "aws")]
impl StoreOptions {
    /// Sign S3 requests for `region`, overriding hints and discovery.
    pub fn with_aws_region(mut self, region: impl Into<String>) -> Self {
        self.aws_region = Some(region.into());
        self
    }

    /// Discover S3 bucket regions through `regions` instead of the process-wide cache.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_bucket_regions(mut self, regions: crate::region::BucketRegions) -> Self {
        self.region_discovery = RegionDiscovery::Custom(regions);
        self
    }

    /// Never probe S3 for a bucket's region.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn without_region_discovery(mut self) -> Self {
        self.region_discovery = RegionDiscovery::Disabled;
        self
    }

    /// The region to sign for, in order: the configured region; the vended hint; the
    /// region of a vended access-point ARN; the bucket's discovered region (AWS only,
    /// native only); the `AWS_REGION` environment variable. `None` leaves object_store's
    /// default.
    async fn s3_region(&self, seed: &VendedCredential) -> Option<String> {
        if let Some(region) = self
            .aws_region
            .clone()
            .or_else(|| seed.hints.region.clone())
        {
            return Some(region);
        }
        if let Some(region) = seed
            .hints
            .access_point
            .as_deref()
            .and_then(access_point_region)
        {
            return Some(region.to_owned());
        }
        // A custom endpoint is an S3-compatible store, not AWS: nothing to probe.
        #[cfg(not(target_arch = "wasm32"))]
        if seed.hints.endpoint.is_none() {
            let regions = match &self.region_discovery {
                RegionDiscovery::Shared => crate::region::BucketRegions::shared(),
                RegionDiscovery::Custom(regions) => Some(regions.clone()),
                RegionDiscovery::Disabled => None,
            };
            if let Some(regions) = regions {
                let bucket = seed.scope.location().bucket();
                if let Some(region) = regions.region(bucket).await {
                    return Some(region);
                }
            }
        }
        std::env::var("AWS_REGION").ok().filter(|r| !r.is_empty())
    }
}

impl fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("StoreOptions");
        #[cfg(any(feature = "aws", feature = "azure", feature = "gcp"))]
        f.field("refresh_margin", &self.refresh_margin)
            .field("clock", &self.clock.as_ref().map(|_| "<fn>"))
            .field("client_options", &self.client_options);
        #[cfg(feature = "aws")]
        f.field("aws_region", &self.aws_region);
        #[cfg(all(feature = "aws", not(target_arch = "wasm32")))]
        f.field("region_discovery", &self.region_discovery);
        f.finish_non_exhaustive()
    }
}

/// Builds the store for `seed`, re-vending from `source` before it expires.
///
/// `seed` must have been vended by `source`: the store is seeded with it, so building
/// does not vend again, and every refresh must still cover `seed`'s scope and access.
///
/// The backend follows `seed`'s location:
///
/// | Location | Store |
/// |---|---|
/// | S3 (`aws`) | `AmazonS3`, signed for the [region](StoreOptions::with_aws_region) it finds; an access-point hint replaces the bucket, an endpoint hint targets an S3-compatible store |
/// | R2 (`aws`) | `AmazonS3` against the R2 account endpoint, region `auto` |
/// | Azure (`azure`) | `MicrosoftAzure`; an endpoint hint targets Azurite or a private endpoint |
/// | GCS (`gcp`) | `GoogleCloudStorage`; an endpoint hint replaces the base URL |
/// | `file://` (native) | `LocalFileSystem` |
/// | `memory://` | a new, empty `InMemory` |
///
/// A cloud location vended [no credential](crate::StorageCredentials::None) builds an
/// unsigned store, for public data.
pub async fn build_store(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<ScopedStore> {
    let location = seed.scope.location();
    let root: Arc<dyn ObjectStore> = match location.provider() {
        StorageProvider::Aws | StorageProvider::R2 => s3(seed, source, options).await?,
        StorageProvider::Azure => azure(seed, source, options)?,
        StorageProvider::Gcp => gcs(seed, source, options)?,
        StorageProvider::Local => local(location)?,
        StorageProvider::Memory => Arc::new(InMemory::new()),
        _ => return Err(unsupported(location, "unknown storage provider")),
    };
    Ok(ScopedStore {
        root,
        prefix: location.path().clone(),
        scope: seed.scope.clone(),
    })
}

fn unsupported(location: &StorageLocation, reason: impl Into<String>) -> Error {
    Error::Unsupported {
        location: location.to_string(),
        reason: reason.into(),
    }
}

#[cfg(feature = "aws")]
async fn s3(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    use object_store::aws::{AmazonS3Builder, AwsCredential};

    let location = seed.scope.location();
    let mut builder = AmazonS3Builder::new().with_client_options(options.client_options(seed));
    builder = if location.provider() == StorageProvider::R2 {
        let account = location
            .account()
            .ok_or_else(|| unsupported(location, "R2 location names no account"))?;
        let endpoint = seed
            .hints
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://{account}.r2.cloudflarestorage.com"));
        // R2 accepts any SigV4 region but documents `auto`; an AWS region override must
        // not apply to it.
        builder
            .with_bucket_name(location.bucket())
            .with_endpoint(endpoint)
            .with_region(seed.hints.region.as_deref().unwrap_or("auto"))
    } else {
        // An access point stands in for the bucket, so signatures match what was granted.
        let bucket = seed
            .hints
            .access_point
            .as_deref()
            .unwrap_or(location.bucket());
        let mut builder = builder.with_bucket_name(bucket);
        if let Some(region) = options.s3_region(seed).await {
            builder = builder.with_region(region);
        }
        if let Some(endpoint) = &seed.hints.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        builder
    };
    if let Some(path_style) = seed.hints.path_style {
        builder = builder.with_virtual_hosted_style_request(!path_style);
    }
    builder = match &seed.credentials {
        StorageCredentials::None => builder.with_skip_signature(true),
        _ => builder.with_credentials(Arc::new(options.provider::<AwsCredential>(seed, source)?)),
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(connector) = options.connector() {
        builder = builder.with_http_connector(connector);
    }
    Ok(Arc::new(builder.build()?))
}

#[cfg(not(feature = "aws"))]
async fn s3(
    _: &VendedCredential,
    _: Arc<dyn CredentialSource>,
    _: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    Err(Error::FeatureDisabled {
        provider: "S3",
        feature: "aws",
    })
}

#[cfg(feature = "azure")]
fn azure(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    use object_store::azure::{AzureCredential, MicrosoftAzureBuilder};

    let location = seed.scope.location();
    let account = location.account().ok_or_else(|| {
        unsupported(
            location,
            "Azure location names no storage account (az://container/...)",
        )
    })?;
    let mut builder = MicrosoftAzureBuilder::new()
        .with_client_options(options.client_options(seed))
        .with_account(account)
        .with_container_name(location.bucket());
    if let Some(endpoint) = &seed.hints.endpoint {
        builder = builder.with_endpoint(azure_account_url(endpoint, account));
    }
    builder = match &seed.credentials {
        StorageCredentials::None => builder.with_skip_signature(true),
        _ => builder.with_credentials(Arc::new(options.provider::<AzureCredential>(seed, source)?)),
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(connector) = options.connector() {
        builder = builder.with_http_connector(connector);
    }
    Ok(Arc::new(builder.build()?))
}

#[cfg(not(feature = "azure"))]
fn azure(
    _: &VendedCredential,
    _: Arc<dyn CredentialSource>,
    _: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    Err(Error::FeatureDisabled {
        provider: "Azure",
        feature: "azure",
    })
}

/// The account URL for an Azure endpoint hint.
///
/// A loopback endpoint is the Azurite emulator, which serves accounts path-style
/// (`http://127.0.0.1:10000/<account>`); any other endpoint already names the account.
/// object_store's own emulator mode is not used: it ignores credential providers, so
/// the store could not refresh.
#[cfg(feature = "azure")]
fn azure_account_url(endpoint: &str, account: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    let loopback = Url::parse(endpoint).is_ok_and(|url| {
        matches!(url.host(), Some(url::Host::Domain("localhost")))
            || matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
            || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
    });
    if loopback && !endpoint.ends_with(&format!("/{account}")) {
        format!("{endpoint}/{account}")
    } else {
        endpoint.to_owned()
    }
}

#[cfg(feature = "gcp")]
fn gcs(
    seed: &VendedCredential,
    source: Arc<dyn CredentialSource>,
    options: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    use object_store::gcp::{GcpCredential, GoogleCloudStorageBuilder};

    let location = seed.scope.location();
    let mut builder = GoogleCloudStorageBuilder::new()
        .with_client_options(options.client_options(seed))
        .with_bucket_name(location.bucket());
    if let Some(endpoint) = &seed.hints.endpoint {
        builder = builder.with_base_url(endpoint);
    }
    builder = match &seed.credentials {
        StorageCredentials::None => builder.with_skip_signature(true),
        _ => builder.with_credentials(Arc::new(options.provider::<GcpCredential>(seed, source)?)),
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(connector) = options.connector() {
        builder = builder.with_http_connector(connector);
    }
    Ok(Arc::new(builder.build()?))
}

#[cfg(not(feature = "gcp"))]
fn gcs(
    _: &VendedCredential,
    _: Arc<dyn CredentialSource>,
    _: &StoreOptions,
) -> Result<Arc<dyn ObjectStore>> {
    Err(Error::FeatureDisabled {
        provider: "GCS",
        feature: "gcp",
    })
}

/// A local file system store, rooted at `/` so paths are absolute like object keys.
///
/// POSIX only: object_store paths cannot express Windows absolute paths.
#[cfg(all(feature = "fs", not(target_arch = "wasm32")))]
fn local(location: &StorageLocation) -> Result<Arc<dyn ObjectStore>> {
    if cfg!(windows) {
        return Err(unsupported(
            location,
            "local storage is not supported on Windows",
        ));
    }
    Ok(Arc::new(object_store::local::LocalFileSystem::new()))
}

#[cfg(not(all(feature = "fs", not(target_arch = "wasm32"))))]
fn local(_: &StorageLocation) -> Result<Arc<dyn ObjectStore>> {
    Err(Error::FeatureDisabled {
        provider: "local file system",
        feature: "fs",
    })
}

/// The region of an S3 access-point ARN (`arn:<partition>:s3:<region>:...`).
#[cfg(feature = "aws")]
fn access_point_region(arn: &str) -> Option<&str> {
    let mut parts = arn.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("arn"), Some(_), Some("s3"), Some(region)) if !region.is_empty() => Some(region),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use object_store::{ObjectStoreExt, PutPayload};
    use olai_storage_credentials::{AccessLevel, StoreHints};

    use super::*;

    /// A source that must never be asked: building a store is seeded, not vended.
    #[derive(Debug)]
    struct NeverVends;

    #[async_trait]
    impl CredentialSource for NeverVends {
        async fn fetch(&self) -> Result<VendedCredential> {
            panic!("building a store must not vend");
        }
    }

    fn seed(url: &str, credentials: StorageCredentials) -> VendedCredential {
        VendedCredential::new(credentials, url.parse().unwrap(), AccessLevel::ReadWrite)
    }

    async fn build(seed: &VendedCredential, options: &StoreOptions) -> Result<ScopedStore> {
        build_store(seed, Arc::new(NeverVends), options).await
    }

    #[cfg(feature = "aws")]
    fn aws() -> StorageCredentials {
        StorageCredentials::aws("AKID", "secret", Some("token".into()))
    }

    #[cfg(feature = "aws")]
    fn no_discovery() -> StoreOptions {
        StoreOptions::new().without_region_discovery()
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn s3_store_is_bucket_rooted_with_the_scope_as_prefix() {
        let store = build(&seed("s3://bucket/tables/t1", aws()), &no_discovery())
            .await
            .unwrap();
        assert_eq!(store.root().to_string(), "AmazonS3(bucket)");
        assert_eq!(store.prefix().as_ref(), "tables/t1");
        assert_eq!(store.url().as_str(), "s3://bucket/tables/t1");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn access_point_replaces_the_bucket() {
        let arn = "arn:aws:s3:eu-west-1:123456789012:accesspoint/ap";
        let seed = seed("s3://bucket/t", aws()).with_hints(StoreHints {
            access_point: Some(arn.into()),
            ..StoreHints::default()
        });
        let store = build(&seed, &no_discovery()).await.unwrap();
        assert_eq!(store.root().to_string(), format!("AmazonS3({arn})"));
        assert_eq!(
            no_discovery().s3_region(&seed).await.as_deref(),
            Some("eu-west-1")
        );
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn r2_store_targets_the_account_endpoint() {
        let store = build(
            &seed("r2://bucket@acct.r2.cloudflarestorage.com/t", aws()),
            // R2 ignores the AWS region override.
            &no_discovery().with_aws_region("us-west-2"),
        )
        .await
        .unwrap();
        assert_eq!(store.root().to_string(), "AmazonS3(bucket)");
        assert_eq!(store.prefix().as_ref(), "t");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn region_precedence() {
        let hinted = seed("s3://bucket/t", aws()).with_hints(StoreHints {
            region: Some("ap-south-1".into()),
            access_point: Some("arn:aws:s3:eu-west-1:1:accesspoint/ap".into()),
            ..StoreHints::default()
        });
        // The configured region beats the hint, which beats the access point.
        let options = no_discovery().with_aws_region("us-west-2");
        assert_eq!(
            options.s3_region(&hinted).await.as_deref(),
            Some("us-west-2")
        );
        assert_eq!(
            no_discovery().s3_region(&hinted).await.as_deref(),
            Some("ap-south-1")
        );
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn region_is_discovered_for_aws_buckets_only() {
        let mut server = mockito::Server::new_async().await;
        let probe = server
            .mock("HEAD", "/bucket")
            .with_status(403)
            .with_header("x-amz-bucket-region", "sa-east-1")
            .expect(1)
            .create_async()
            .await;
        let regions = crate::region::BucketRegions::with_endpoint(server.url()).unwrap();
        let options = StoreOptions::new().with_bucket_regions(regions);

        let plain = seed("s3://bucket/t", aws());
        assert_eq!(
            options.s3_region(&plain).await.as_deref(),
            Some("sa-east-1")
        );

        // An S3-compatible endpoint is not AWS: no probe.
        let compatible = plain.clone().with_hints(StoreHints {
            endpoint: Some("http://127.0.0.1:9000".into()),
            ..StoreHints::default()
        });
        let region = options.s3_region(&compatible).await;
        assert_ne!(region.as_deref(), Some("sa-east-1"));
        probe.assert_async().await;
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn public_data_builds_an_unsigned_store() {
        let store = build(
            &seed("s3://bucket/t", StorageCredentials::None),
            &no_discovery(),
        )
        .await;
        assert!(store.is_ok(), "{store:?}");
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn credential_for_another_cloud_is_rejected() {
        let err = build(
            &seed(
                "s3://bucket/t",
                StorageCredentials::GcpBearer("ya29".into()),
            ),
            &no_discovery(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::CredentialKind { .. }), "{err}");
    }

    #[cfg(feature = "azure")]
    #[tokio::test]
    async fn azure_store_needs_an_account() {
        let sas = StorageCredentials::azure_sas("sv=2022-11-02&sig=abc").unwrap();
        let store = build(
            &seed(
                "abfss://ctr@acct.dfs.core.windows.net/tables/t1",
                sas.clone(),
            ),
            &StoreOptions::new(),
        )
        .await
        .unwrap();
        assert_eq!(store.prefix().as_ref(), "tables/t1");
        assert!(store.root().to_string().contains("ctr"), "{store:?}");

        let err = build(&seed("az://ctr/t", sas), &StoreOptions::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[cfg(feature = "azure")]
    #[test]
    fn azurite_endpoints_are_path_style() {
        let account = "devstoreaccount1";
        for (endpoint, expected) in [
            (
                "http://127.0.0.1:10000",
                "http://127.0.0.1:10000/devstoreaccount1",
            ),
            (
                "https://localhost:4000/",
                "https://localhost:4000/devstoreaccount1",
            ),
            ("http://[::1]:10000", "http://[::1]:10000/devstoreaccount1"),
            (
                "https://127.0.0.1:4000/devstoreaccount1",
                "https://127.0.0.1:4000/devstoreaccount1",
            ),
            (
                "https://acct.privatelink.blob.core.windows.net",
                "https://acct.privatelink.blob.core.windows.net",
            ),
        ] {
            assert_eq!(azure_account_url(endpoint, account), expected, "{endpoint}");
        }
    }

    #[cfg(feature = "gcp")]
    #[tokio::test]
    async fn gcs_store() {
        let store = build(
            &seed(
                "gs://bucket/t",
                StorageCredentials::GcpBearer("ya29".into()),
            ),
            &StoreOptions::new(),
        )
        .await
        .unwrap();
        assert_eq!(store.root().to_string(), "GoogleCloudStorage(bucket)");
        assert_eq!(store.prefix().as_ref(), "t");
    }

    /// The vended token signs requests to the endpoint hint, which is what an emulator
    /// such as fake-gcs-server relies on.
    #[cfg(feature = "gcp")]
    #[tokio::test]
    async fn gcs_endpoint_hint_receives_the_vended_token() {
        let mut server = mockito::Server::new_async().await;
        let get = server
            // object_store percent-encodes the object name into one segment.
            .mock("GET", "/bucket/t%2Fpart%2D0")
            .match_header("authorization", "Bearer ya29-vended")
            .with_status(200)
            .with_header("last-modified", "Thu, 01 Jan 2026 00:00:00 GMT")
            .with_header("etag", "\"1\"")
            .with_body("x")
            .create_async()
            .await;
        let seed = seed(
            "gs://bucket/t",
            StorageCredentials::GcpBearer("ya29-vended".into()),
        )
        .with_hints(StoreHints {
            endpoint: Some(server.url()),
            allow_http: Some(true),
            ..StoreHints::default()
        });
        let store = build(&seed, &StoreOptions::new()).await.unwrap();
        let body = store
            .prefixed()
            .get(&"part-0".into())
            .await
            .unwrap()
            .bytes()
            .await;
        assert_eq!(body.unwrap().as_ref(), b"x");
        get.assert_async().await;
    }

    #[tokio::test]
    async fn memory_store_prefixed_view() {
        let store = build(
            &seed("memory:///tables/t1", StorageCredentials::None),
            &StoreOptions::new(),
        )
        .await
        .unwrap();
        store
            .prefixed()
            .put(&"part-0".into(), PutPayload::from_static(b"x"))
            .await
            .unwrap();
        // The prefixed view writes under the scope of the bucket-rooted store.
        store.root().head(&"tables/t1/part-0".into()).await.unwrap();
    }

    #[cfg(all(feature = "fs", unix))]
    #[tokio::test]
    async fn local_store_uses_absolute_paths() {
        let dir = std::env::temp_dir().join(format!("olai-vended-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = Url::from_directory_path(&dir).unwrap();
        let store = build(
            &seed(url.as_str(), StorageCredentials::None),
            &StoreOptions::new(),
        )
        .await
        .unwrap();
        store
            .prefixed()
            .put(&"f".into(), PutPayload::from_static(b"x"))
            .await
            .unwrap();
        assert!(dir.join("f").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn debug_does_not_leak() {
        let options = StoreOptions::new();
        assert!(format!("{options:?}").starts_with("StoreOptions"));
    }
}
