//! Stores built from vended credentials, against emulators that enforce them.
//!
//! - RustFS: a credential minted by STS with a session policy for one prefix. The store
//!   reads and writes inside the prefix, is denied on a sibling prefix (`tab` vs
//!   `table2`), and keeps working across refreshes.
//! - Azurite: a user-delegation SAS for one container, through the emulator's
//!   path-style endpoint.
//! - Routing and presigned URLs over both, fetched with a plain HTTP client.
//!
//! Needs Docker; run with `cargo test -p olai-vended-store --features emulator-it`.
#![cfg(feature = "emulator-it")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures::TryStreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use olai_http::aws::{AwsCredential, assume_role_with_base_and_options};
use olai_http::azure::{UserDelegationKeyOptions, generate_user_delegation_sas_with_options};
use olai_http::{ClientOptions, StaticCredentialProvider};
use olai_storage_credentials::policy::s3::session_policy;
use olai_test_emulators::azurite::ACCOUNT;
use olai_test_emulators::rustfs::{REGION, ROOT_ACCESS_KEY, ROOT_SECRET_KEY};
use olai_test_emulators::{Azurite, RustFs};
use olai_vended_store::{
    AccessLevel, CredentialSource, Result, RoutingObjectStore, StorageCredentials, StorageLocation,
    StorageScope, StoreHints, StoreOptions, VendedCredential, build_store, presign,
};
use reqwest::Method;

const BUCKET: &str = "vend";

/// Vends from RustFS STS, downscoped to `scope`, and counts vends.
#[derive(Debug)]
struct RustFsSource {
    endpoint: String,
    scope: StorageScope,
    access: AccessLevel,
    vends: AtomicUsize,
}

#[async_trait]
impl CredentialSource for RustFsSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        self.vends.fetch_add(1, Ordering::SeqCst);
        let root = Arc::new(StaticCredentialProvider::new(AwsCredential {
            key_id: ROOT_ACCESS_KEY.to_owned(),
            secret_key: ROOT_SECRET_KEY.to_owned(),
            token: None,
        }));
        let session = assume_role_with_base_and_options(
            "arn:aws:iam::000000000000:role/vending",
            REGION,
            Some(&self.endpoint),
            Some(session_policy(&self.scope, self.access)?),
            root,
            &ClientOptions::new().with_allow_http(true),
        )
        .await
        .map_err(olai_vended_store::Error::vend)?;
        let creds = session.token;
        Ok(VendedCredential::new(
            StorageCredentials::aws(&creds.key_id, &creds.secret_key, creds.token.clone()),
            self.scope.clone(),
            self.access,
        )
        .with_expiry(Some(Utc::now() + Duration::from_secs(3600)))
        .with_hints(StoreHints {
            endpoint: Some(self.endpoint.clone()),
            region: Some(REGION.to_owned()),
            path_style: Some(true),
            allow_http: Some(true),
            ..StoreHints::default()
        }))
    }
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

fn is_denied(err: &object_store::Error) -> bool {
    matches!(
        err,
        object_store::Error::PermissionDenied { .. } | object_store::Error::Unauthenticated { .. }
    ) || err.to_string().contains("403")
}

#[tokio::test]
async fn s3_store_is_confined_to_the_vended_prefix() {
    let rustfs = rustfs().await;
    let source = Arc::new(RustFsSource {
        endpoint: rustfs.endpoint().to_owned(),
        scope: format!("s3://{BUCKET}/tab").parse().unwrap(),
        access: AccessLevel::ReadWrite,
        vends: AtomicUsize::new(0),
    });
    let seed = source.fetch().await.unwrap();
    let store = build_store(&seed, source.clone(), &StoreOptions::new())
        .await
        .unwrap();
    assert_eq!(store.prefix().as_ref(), "tab");

    // Inside the prefix: read, list, write, through either view.
    let root = store.root();
    root.get(&Path::from("tab/a")).await.unwrap();
    store
        .prefixed()
        .put(&Path::from("new"), PutPayload::from_static(b"y"))
        .await
        .unwrap();
    let listed: Vec<_> = store.prefixed().list(None).try_collect().await.unwrap();
    let mut names: Vec<_> = listed.iter().map(|m| m.location.to_string()).collect();
    names.sort();
    assert_eq!(names, ["a", "new"]);

    // A sibling prefix sharing the string prefix is outside the grant.
    let err = root.get(&Path::from("table2/b")).await.unwrap_err();
    assert!(is_denied(&err), "read outside the prefix: {err}");
    let err = root
        .put(&Path::from("table2/c"), PutPayload::from_static(b"z"))
        .await
        .unwrap_err();
    assert!(is_denied(&err), "write outside the prefix: {err}");

    // Building the store was seeded: only the explicit fetch above vended.
    assert_eq!(source.vends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn s3_store_refreshes_with_fresh_credentials() {
    let rustfs = rustfs().await;
    let source = Arc::new(RustFsSource {
        endpoint: rustfs.endpoint().to_owned(),
        scope: format!("s3://{BUCKET}/tab").parse().unwrap(),
        access: AccessLevel::Read,
        vends: AtomicUsize::new(0),
    });
    let seed = source.fetch().await.unwrap();
    // A margin longer than the credential's lifetime makes every request due; the
    // halfway rule then refreshes once per request.
    let options = StoreOptions::new().with_refresh_margin(Duration::from_secs(7200));
    let clock_offset = Arc::new(AtomicUsize::new(0));
    let offset = clock_offset.clone();
    let options = options.with_clock(Arc::new(move || {
        Utc::now() + Duration::from_secs(offset.load(Ordering::SeqCst) as u64)
    }));
    let store = build_store(&seed, source.clone(), &options).await.unwrap();

    store.root().get(&Path::from("tab/a")).await.unwrap();
    assert_eq!(source.vends.load(Ordering::SeqCst), 1, "seed still fresh");

    // Past the halfway point: the next request re-vends and still works.
    clock_offset.store(1801, Ordering::SeqCst);
    store.root().get(&Path::from("tab/a")).await.unwrap();
    assert_eq!(source.vends.load(Ordering::SeqCst), 2);
}

/// Vends a user-delegation SAS for one Azurite container.
#[derive(Debug)]
struct AzuriteSource {
    endpoint: String,
    bearer: String,
    scope: StorageScope,
}

#[async_trait]
impl CredentialSource for AzuriteSource {
    async fn fetch(&self) -> Result<VendedCredential> {
        let options = UserDelegationKeyOptions {
            endpoint: Some(self.endpoint.clone()),
            client_options: ClientOptions::new().with_allow_invalid_certificates(true),
        };
        let sas = generate_user_delegation_sas_with_options(
            ACCOUNT,
            self.scope.location().bucket(),
            "",
            &self.bearer,
            false,
            3600,
            &options,
        )
        .await
        .map_err(olai_vended_store::Error::vend)?;
        Ok(VendedCredential::new(
            StorageCredentials::azure_sas(&sas)?,
            self.scope.clone(),
            AccessLevel::ReadWrite,
        )
        .with_expiry(Some(Utc::now() + Duration::from_secs(3600)))
        .with_hints(StoreHints {
            endpoint: Some(self.endpoint.clone()),
            ..StoreHints::default()
        }))
    }
}

#[tokio::test]
async fn azure_store_through_the_azurite_endpoint() {
    let azurite = Azurite::start().await;
    azurite.create_container("granted").await;
    azurite.create_container("other").await;
    azurite.put_blob("granted", "tab/a", b"x").await;
    azurite.put_blob("other", "tab/a", b"x").await;

    let source = Arc::new(AzuriteSource {
        endpoint: azurite.blob_endpoint().to_owned(),
        bearer: azurite.bearer_token(),
        scope: format!("abfss://granted@{ACCOUNT}.dfs.core.windows.net/tab")
            .parse()
            .unwrap(),
    });
    let seed = source.fetch().await.unwrap();
    let options = StoreOptions::new().with_client_options(
        object_store::ClientOptions::new().with_allow_invalid_certificates(true),
    );
    let store = build_store(&seed, source.clone(), &options).await.unwrap();

    store.prefixed().get(&Path::from("a")).await.unwrap();
    store
        .prefixed()
        .put(&Path::from("b"), PutPayload::from_static(b"y"))
        .await
        .unwrap();

    // The SAS is for one container; a store for another container is refused.
    let mut other = seed.clone();
    other.scope = format!("abfss://other@{ACCOUNT}.dfs.core.windows.net/tab")
        .parse()
        .unwrap();
    let foreign = build_store(&other, source, &options).await.unwrap();
    let err = foreign.prefixed().get(&Path::from("a")).await.unwrap_err();
    assert!(is_denied(&err), "read in another container: {err}");
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        // Azurite serves a per-run self-signed certificate.
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
}

/// The full client path: vend, build, route, presign.
#[tokio::test]
async fn s3_vend_route_and_presign() {
    let rustfs = rustfs().await;
    let source = Arc::new(RustFsSource {
        endpoint: rustfs.endpoint().to_owned(),
        scope: format!("s3://{BUCKET}/tab").parse().unwrap(),
        access: AccessLevel::ReadWrite,
        vends: AtomicUsize::new(0),
    });
    let seed = source.fetch().await.unwrap();
    let store = build_store(&seed, source.clone(), &StoreOptions::new())
        .await
        .unwrap();

    let router = RoutingObjectStore::new();
    router.register(&store).unwrap();
    router
        .put(
            &Path::from("tab/routed"),
            PutPayload::from_static(b"via router"),
        )
        .await
        .unwrap();
    assert!(
        router.get(&Path::from("table2/b")).await.is_err(),
        "no route"
    );

    let location: StorageLocation = format!("s3://{BUCKET}/tab/routed").parse().unwrap();
    let signed = presign(
        &seed,
        Method::GET,
        std::slice::from_ref(&location),
        Duration::from_secs(60),
        &StoreOptions::new(),
    )
    .await
    .unwrap();
    let response = http().get(signed.urls[0].clone()).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"via router");

    // The signature binds the path: pointing the URL at another key fails.
    let mut tampered = signed.urls[0].clone();
    tampered.set_path(&format!("/{BUCKET}/table2/b"));
    assert_eq!(http().get(tampered).send().await.unwrap().status(), 403);
}

#[tokio::test]
async fn azure_presigned_urls_are_accepted() {
    let azurite = Azurite::start().await;
    azurite.create_container("granted").await;
    azurite.put_blob("granted", "tab/a", b"x").await;
    let scope = format!("abfss://granted@{ACCOUNT}.dfs.core.windows.net/tab");
    let location: StorageLocation = format!("{scope}/a").parse().unwrap();
    let options = StoreOptions::new().with_client_options(
        object_store::ClientOptions::new().with_allow_invalid_certificates(true),
    );

    // A vended SAS, appended to the blob URL.
    let source = AzuriteSource {
        endpoint: azurite.blob_endpoint().to_owned(),
        bearer: azurite.bearer_token(),
        scope: scope.parse().unwrap(),
    };
    let sas = source.fetch().await.unwrap();
    let signed = presign(
        &sas,
        Method::GET,
        std::slice::from_ref(&location),
        Duration::from_secs(60),
        &options,
    )
    .await
    .unwrap();
    assert_eq!(
        http()
            .get(signed.urls[0].clone())
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    // An AD token, from which a user-delegation SAS is signed.
    let bearer = VendedCredential::new(
        StorageCredentials::AzureBearer(azurite.bearer_token()),
        scope.parse().unwrap(),
        AccessLevel::Read,
    )
    .with_expiry(Some(Utc::now() + Duration::from_secs(3600)))
    .with_hints(sas.hints.clone());
    let signed = presign(
        &bearer,
        Method::GET,
        std::slice::from_ref(&location),
        Duration::from_secs(60),
        &options,
    )
    .await
    .unwrap();
    assert_eq!(
        http()
            .get(signed.urls[0].clone())
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}
