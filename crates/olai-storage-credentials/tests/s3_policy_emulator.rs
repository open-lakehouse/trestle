//! Proves [`session_policy`] downscopes for real: mint session credentials from RustFS's
//! STS with the policy attached, then check what an S3 client can and cannot do with them.
//!
//! Needs Docker; run with `cargo test -p olai-storage-credentials --features emulator-it`.
#![cfg(feature = "emulator-it")]

use std::sync::Arc;

use futures::TryStreamExt;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use olai_http::aws::{AwsCredential, assume_role_with_base_and_options};
use olai_http::{ClientOptions, StaticCredentialProvider};
use olai_storage_credentials::policy::s3::session_policy;
use olai_storage_credentials::{AccessLevel, StorageScope};
use olai_test_emulators::RustFs;
use olai_test_emulators::rustfs::{REGION, ROOT_ACCESS_KEY, ROOT_SECRET_KEY};

const BUCKET: &str = "vend";
const SEEDED: &[&str] = &["tab/a", "tab/sub/b", "table2/c", "other/d"];

fn store(rustfs: &RustFs, key_id: &str, secret: &str, token: Option<&str>) -> AmazonS3 {
    let mut builder = AmazonS3Builder::new()
        .with_endpoint(rustfs.endpoint())
        .with_allow_http(true)
        .with_region(REGION)
        .with_bucket_name(BUCKET)
        .with_access_key_id(key_id)
        .with_secret_access_key(secret);
    if let Some(token) = token {
        builder = builder.with_token(token);
    }
    builder.build().expect("build S3 store")
}

async fn setup() -> RustFs {
    let rustfs = RustFs::start().await;
    rustfs.create_bucket(BUCKET).await;
    let root = store(&rustfs, ROOT_ACCESS_KEY, ROOT_SECRET_KEY, None);
    for key in SEEDED {
        root.put(&Path::from(*key), PutPayload::from_static(b"x"))
            .await
            .unwrap_or_else(|e| panic!("seed {key}: {e}"));
    }
    rustfs
}

/// A store holding session credentials minted for `scope` at `access`.
async fn scoped_store(rustfs: &RustFs, scope: &str, access: AccessLevel) -> AmazonS3 {
    let scope: StorageScope = scope.parse().unwrap();
    let policy = session_policy(&scope, access).unwrap();
    let root = Arc::new(StaticCredentialProvider::new(AwsCredential {
        key_id: ROOT_ACCESS_KEY.to_owned(),
        secret_key: ROOT_SECRET_KEY.to_owned(),
        token: None,
    }));
    let session = assume_role_with_base_and_options(
        // RustFS does not resolve roles; the session takes the caller's identity,
        // narrowed by the policy.
        "arn:aws:iam::000000000000:role/vending",
        REGION,
        Some(rustfs.endpoint()),
        Some(policy),
        root,
        &ClientOptions::new().with_allow_http(true),
    )
    .await
    .expect("AssumeRole against RustFS STS");
    let creds = session.token;
    assert!(creds.token.is_some(), "STS returns a session token");
    store(
        rustfs,
        &creds.key_id,
        &creds.secret_key,
        creds.token.as_deref(),
    )
}

/// Whether `err` is an authorization failure. object_store maps a 403 on GET/PUT to
/// `PermissionDenied`, but reports list and batch-delete failures as `Generic` with the
/// S3 `AccessDenied` error in the message.
fn is_denied(err: &object_store::Error) -> bool {
    match err {
        object_store::Error::PermissionDenied { .. }
        | object_store::Error::Unauthenticated { .. } => true,
        object_store::Error::Generic { .. } => {
            let message = err.to_string();
            message.contains("AccessDenied") || message.contains("403 Forbidden")
        }
        _ => false,
    }
}

async fn assert_get(store: &AmazonS3, key: &str, allowed: bool) {
    match store.get(&Path::from(key)).await {
        Ok(_) => assert!(allowed, "GET {key} should be denied"),
        Err(e) => assert!(!allowed && is_denied(&e), "GET {key}: unexpected {e}"),
    }
}

async fn assert_list(store: &AmazonS3, prefix: Option<&str>, allowed: bool) {
    let prefix = prefix.map(Path::from);
    let listed = store.list(prefix.as_ref()).try_collect::<Vec<_>>().await;
    match listed {
        Ok(objects) => {
            assert!(allowed, "LIST {prefix:?} should be denied");
            assert!(!objects.is_empty(), "LIST {prefix:?} returned nothing");
        }
        Err(e) => assert!(!allowed && is_denied(&e), "LIST {prefix:?}: unexpected {e}"),
    }
}

async fn assert_put(store: &AmazonS3, key: &str, allowed: bool) {
    match store
        .put(&Path::from(key), PutPayload::from_static(b"y"))
        .await
    {
        Ok(_) => assert!(allowed, "PUT {key} should be denied"),
        Err(e) => assert!(!allowed && is_denied(&e), "PUT {key}: unexpected {e}"),
    }
}

#[tokio::test]
async fn read_policy_confines_reads_and_listing_to_the_prefix() {
    let rustfs = setup().await;
    let store = scoped_store(&rustfs, "s3://vend/tab", AccessLevel::Read).await;

    assert_get(&store, "tab/a", true).await;
    assert_get(&store, "tab/sub/b", true).await;
    // `tab` must not cover the sibling `table2`.
    assert_get(&store, "table2/c", false).await;
    assert_get(&store, "other/d", false).await;

    assert_list(&store, Some("tab"), true).await;
    assert_list(&store, Some("tab/sub"), true).await;
    assert_list(&store, None, false).await;
    assert_list(&store, Some("table2"), false).await;

    assert_put(&store, "tab/new", false).await;
}

#[tokio::test]
async fn read_write_policy_confines_writes_to_the_prefix() {
    let rustfs = setup().await;
    let store = scoped_store(&rustfs, "s3://vend/tab/", AccessLevel::ReadWrite).await;

    assert_put(&store, "tab/new", true).await;
    assert_put(&store, "table2/new", false).await;
    assert_put(&store, "other/new", false).await;

    store
        .delete(&Path::from("tab/new"))
        .await
        .expect("delete inside the prefix");
    let err = store.delete(&Path::from("other/d")).await.unwrap_err();
    assert!(is_denied(&err), "delete outside the prefix: {err}");

    // Multipart: a full upload (first part at the 5 MiB minimum) and an abort.
    let mut upload = store
        .put_multipart(&Path::from("tab/big"))
        .await
        .expect("start multipart inside the prefix");
    upload
        .put_part(PutPayload::from(vec![0u8; 5 * 1024 * 1024]))
        .await
        .expect("upload part");
    upload
        .put_part(PutPayload::from_static(b"tail"))
        .await
        .expect("upload part");
    upload.complete().await.expect("complete multipart");
    assert_get(&store, "tab/big", true).await;

    let mut aborted = store
        .put_multipart(&Path::from("tab/aborted"))
        .await
        .expect("start multipart");
    aborted
        .put_part(PutPayload::from_static(b"part"))
        .await
        .expect("upload part");
    aborted
        .abort()
        .await
        .expect("abort multipart inside the prefix");

    let err = match store.put_multipart(&Path::from("other/big")).await {
        Ok(mut upload) => upload
            .put_part(PutPayload::from_static(b"x"))
            .await
            .unwrap_err(),
        Err(e) => e,
    };
    assert!(is_denied(&err), "multipart outside the prefix: {err}");
}

#[tokio::test]
async fn bucket_scope_grants_the_whole_bucket() {
    let rustfs = setup().await;
    let store = scoped_store(&rustfs, "s3://vend", AccessLevel::Read).await;

    for key in SEEDED {
        assert_get(&store, key, true).await;
    }
    assert_list(&store, None, true).await;
}
