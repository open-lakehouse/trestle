//! Server and client, end to end: what the server side mints, the client side stores
//! use, against emulators that enforce the grant.
//!
//! Each vend runs the full path a Unity Catalog server and client would:
//!
//! 1. the server resolves the registered credential governing the requested location
//!    and mints through it ([`Vendor`]);
//! 2. it serializes the credential as a UC REST `temporary-*-credentials` response;
//! 3. the client parses the response in the context of its request
//!    ([`TemporaryCredential::to_vended`]) and builds a refreshing store from it
//!    ([`build_store`]).
//!
//! Needs Docker; run with `cargo test -p olai-credential-vending --features emulator-it`.
#![cfg(feature = "emulator-it")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload};
use olai_credential_vending::registered::{AwsIamRole, AzureStorageKey};
use olai_credential_vending::{
    AccessLevel, AwsStsMinter, AzureAccountKeySasMinter, CredentialMinter, ExternalLocations,
    MintRequest, RegisteredCredential, Vendor,
};
use olai_http::ClientOptions;
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::wire::unity::TemporaryCredential;
use olai_test_emulators::azurite::{ACCOUNT, ACCOUNT_KEY};
use olai_test_emulators::rustfs::{REGION, ROOT_ACCESS_KEY, ROOT_SECRET_KEY};
use olai_test_emulators::{Azurite, RustFs};
use olai_vended_store::{
    CredentialSource, RoutingObjectStore, StoreOptions, VendedCredential, build_store,
};

const BUCKET: &str = "vend";

/// The client's view of a vending endpoint: each fetch is one round trip through the
/// server's [`Vendor`] and the UC REST wire format.
#[derive(Debug)]
struct Endpoint {
    vendor: Arc<Vendor<ExternalLocations>>,
    request: MintRequest,
    /// An endpoint hint UC REST cannot carry, which a client would take from the
    /// storage URL (here, Azurite on a random port).
    endpoint: Option<String>,
    vends: AtomicUsize,
}

impl Endpoint {
    fn new(vendor: &Arc<Vendor<ExternalLocations>>, url: &str, access: AccessLevel) -> Arc<Self> {
        Arc::new(Self {
            vendor: Arc::clone(vendor),
            request: MintRequest::new(url.parse().unwrap(), access).with_session_name("alice@e2e"),
            endpoint: None,
            vends: AtomicUsize::new(0),
        })
    }

    fn vends(&self) -> usize {
        self.vends.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CredentialSource for Endpoint {
    async fn fetch(&self) -> olai_vended_store::Result<VendedCredential> {
        self.vends.fetch_add(1, Ordering::SeqCst);
        // Server: resolve, mint, serialize.
        let minted = self
            .vendor
            .vend(&self.request)
            .await
            .map_err(olai_vended_store::Error::vend)?;
        let body = serde_json::to_string(&TemporaryCredential::from(&minted)).unwrap();
        // Client: parse the response for the request it sent.
        let wire: TemporaryCredential = serde_json::from_str(&body).unwrap();
        let mut vended = wire.to_vended(&VendContext::new(
            self.request.scope.clone(),
            self.request.access,
        ))?;
        if vended.hints.endpoint.is_none() {
            vended.hints.endpoint = self.endpoint.clone();
        }
        Ok(vended)
    }
}

fn denied(err: &object_store::Error) -> bool {
    matches!(
        err,
        object_store::Error::PermissionDenied { .. } | object_store::Error::Unauthenticated { .. }
    ) || err.to_string().contains("403")
}

async fn rustfs() -> RustFs {
    let rustfs = RustFs::start().await;
    rustfs.create_bucket(BUCKET).await;
    let root = AmazonS3Builder::new()
        .with_endpoint(rustfs.endpoint())
        .with_allow_http(true)
        .with_region(REGION)
        .with_bucket_name(BUCKET)
        .with_access_key_id(ROOT_ACCESS_KEY)
        .with_secret_access_key(ROOT_SECRET_KEY)
        .build()
        .unwrap();
    for key in ["tab/a", "table2/b"] {
        root.put(&Path::from(key), PutPayload::from_static(b"x"))
            .await
            .unwrap();
    }
    rustfs
}

/// A server whose bucket is governed by an IAM role, assumed through RustFS STS.
fn s3_vendor(rustfs: &RustFs) -> Arc<Vendor<ExternalLocations>> {
    let locations = ExternalLocations::new().with_location(
        format!("s3://{BUCKET}").parse().unwrap(),
        RegisteredCredential::AwsIamRole(AwsIamRole {
            role_arn: "arn:aws:iam::000000000000:role/vending".into(),
            region: Some(REGION.into()),
            access_key_id: Some(ROOT_ACCESS_KEY.into()),
            secret_access_key: Some(ROOT_SECRET_KEY.into()),
            ..AwsIamRole::default()
        }),
    );
    let endpoint = rustfs.endpoint().to_owned();
    Arc::new(Vendor::new(locations).with_minters(move |credential| {
        let RegisteredCredential::AwsIamRole(role) = credential else {
            panic!("only an IAM role is registered");
        };
        let minter: Arc<dyn CredentialMinter> = Arc::new(
            AwsStsMinter::new(role)?
                .with_sts_endpoint(&endpoint)
                .with_client_options(ClientOptions::new().with_allow_http(true)),
        );
        Ok(minter)
    }))
}

/// The client's store options for RustFS: the S3 endpoint UC REST cannot name.
fn s3_options(rustfs: &RustFs) -> StoreOptions {
    StoreOptions::new()
        .with_s3_endpoint(rustfs.endpoint())
        .with_aws_region(REGION)
        .with_client_options(object_store::ClientOptions::new().with_allow_http(true))
}

#[tokio::test]
async fn minted_s3_credentials_serve_client_stores() {
    let rustfs = rustfs().await;
    let vendor = s3_vendor(&rustfs);
    let source = Endpoint::new(
        &vendor,
        &format!("s3://{BUCKET}/tab"),
        AccessLevel::ReadWrite,
    );
    let seed = source.fetch().await.unwrap();
    let store = build_store(&seed, source.clone(), &s3_options(&rustfs))
        .await
        .unwrap();

    // Inside the grant: read, write and list, through a router as a query engine would.
    let router = RoutingObjectStore::new();
    router.register(&store).unwrap();
    router.get(&Path::from("tab/a")).await.unwrap();
    router
        .put(&Path::from("tab/new"), PutPayload::from_static(b"y"))
        .await
        .unwrap();
    assert_eq!(
        store
            .prefixed()
            .list_with_delimiter(None)
            .await
            .unwrap()
            .objects
            .len(),
        2
    );

    // Outside it: the cloud refuses, even through the bucket-rooted store.
    let err = store.root().get(&Path::from("table2/b")).await.unwrap_err();
    assert!(denied(&err), "read on a sibling prefix: {err}");
    let err = store
        .root()
        .put(&Path::from("table2/c"), PutPayload::from_static(b"z"))
        .await
        .unwrap_err();
    assert!(denied(&err), "write on a sibling prefix: {err}");
    assert_eq!(source.vends(), 1, "building the store is seeded");
}

#[tokio::test]
async fn client_refreshes_go_back_through_the_server() {
    let rustfs = rustfs().await;
    let vendor = s3_vendor(&rustfs);
    let source = Endpoint::new(&vendor, &format!("s3://{BUCKET}/tab"), AccessLevel::Read);
    let seed = source.fetch().await.unwrap();
    let offset = Arc::new(AtomicU64::new(0));
    let clock_offset = Arc::clone(&offset);
    let options = s3_options(&rustfs).with_clock(Arc::new(move || {
        Utc::now() + Duration::from_secs(clock_offset.load(Ordering::SeqCst))
    }));
    let store = build_store(&seed, source.clone(), &options).await.unwrap();
    store.root().get(&Path::from("tab/a")).await.unwrap();
    assert_eq!(source.vends(), 1);

    // Near expiry, the client re-vends through the server and keeps working.
    let lifetime = (seed.expires_at.unwrap() - Utc::now()).num_seconds() as u64;
    offset.store(lifetime - 60, Ordering::SeqCst);
    store.root().get(&Path::from("tab/a")).await.unwrap();
    assert_eq!(source.vends(), 2);
    let err = store.root().get(&Path::from("table2/b")).await.unwrap_err();
    assert!(denied(&err), "the refreshed grant is no wider: {err}");
}

#[tokio::test]
async fn unregistered_locations_are_not_vended() {
    let rustfs = rustfs().await;
    let vendor = s3_vendor(&rustfs);
    let source = Endpoint::new(&vendor, "s3://elsewhere/tab", AccessLevel::Read);
    let err = source.fetch().await.unwrap_err();
    assert!(err.to_string().contains("no registered location"), "{err}");
}

#[tokio::test]
async fn minted_azure_sas_serve_client_stores() {
    let azurite = Azurite::start().await;
    for container in ["granted", "other"] {
        azurite.create_container(container).await;
        azurite.put_blob(container, "tab/a", b"x").await;
    }
    let locations = ExternalLocations::new().with_location(
        format!("abfss://granted@{ACCOUNT}.dfs.core.windows.net")
            .parse()
            .unwrap(),
        RegisteredCredential::AzureStorageKey(AzureStorageKey {
            account_name: ACCOUNT.into(),
            account_key: ACCOUNT_KEY.into(),
        }),
    );
    let vendor = Arc::new(Vendor::new(locations).with_minters(|credential| {
        let RegisteredCredential::AzureStorageKey(key) = credential else {
            panic!("only an account key is registered");
        };
        let minter: Arc<dyn CredentialMinter> =
            Arc::new(AzureAccountKeySasMinter::new(key).with_emulator());
        Ok(minter)
    }));
    let source = Arc::new(Endpoint {
        endpoint: Some(azurite.blob_endpoint().to_owned()),
        ..Arc::into_inner(Endpoint::new(
            &vendor,
            &format!("abfss://granted@{ACCOUNT}.dfs.core.windows.net"),
            AccessLevel::ReadWrite,
        ))
        .unwrap()
    });
    let seed = source.fetch().await.unwrap();
    let options = StoreOptions::new().with_client_options(
        object_store::ClientOptions::new().with_allow_invalid_certificates(true),
    );
    let store = build_store(&seed, source.clone(), &options).await.unwrap();

    store.prefixed().get(&Path::from("tab/a")).await.unwrap();
    store
        .prefixed()
        .put(&Path::from("tab/new"), PutPayload::from_static(b"y"))
        .await
        .unwrap();

    // The SAS is for its container only: a store for another container is refused.
    let mut foreign = seed.clone();
    foreign.scope = format!("abfss://other@{ACCOUNT}.dfs.core.windows.net")
        .parse()
        .unwrap();
    let foreign = build_store(&foreign, source, &options).await.unwrap();
    let err = foreign
        .prefixed()
        .get(&Path::from("tab/a"))
        .await
        .unwrap_err();
    assert!(denied(&err), "read in another container: {err}");
}
