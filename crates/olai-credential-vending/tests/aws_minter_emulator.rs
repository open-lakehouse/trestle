//! `AwsStsMinter` against RustFS, whose STS enforces session policies: a minted
//! credential reaches exactly the requested scope at the requested access.
//!
//! Needs Docker; run with `cargo test -p olai-credential-vending --features emulator-it`.
#![cfg(feature = "emulator-it")]

use std::time::Duration;

use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path;
use object_store::{ObjectStoreExt, PutPayload};
use olai_credential_vending::registered::AwsIamRole;
use olai_credential_vending::{
    AccessLevel, AwsStsMinter, CredentialMinter, MintRequest, StorageCredentials, VendedCredential,
};
use olai_http::ClientOptions;
use olai_test_emulators::RustFs;
use olai_test_emulators::rustfs::{REGION, ROOT_ACCESS_KEY, ROOT_SECRET_KEY};

const BUCKET: &str = "vend";

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
    builder.build().unwrap()
}

fn vended_store(rustfs: &RustFs, vended: &VendedCredential) -> AmazonS3 {
    let StorageCredentials::Aws {
        key_id,
        secret_key,
        token,
    } = &vended.credentials
    else {
        panic!("STS mints AWS credentials");
    };
    store(rustfs, key_id, secret_key, token.as_deref())
}

async fn setup() -> (RustFs, AwsStsMinter) {
    let rustfs = RustFs::start().await;
    rustfs.create_bucket(BUCKET).await;
    let root = store(&rustfs, ROOT_ACCESS_KEY, ROOT_SECRET_KEY, None);
    for key in ["tab/a", "table2/b"] {
        root.put(&Path::from(key), PutPayload::from_static(b"x"))
            .await
            .unwrap();
    }
    let minter = AwsStsMinter::new(&AwsIamRole {
        // RustFS does not resolve roles; the session takes the caller's identity,
        // narrowed by the policy.
        role_arn: "arn:aws:iam::000000000000:role/vending".into(),
        region: Some(REGION.into()),
        access_key_id: Some(ROOT_ACCESS_KEY.into()),
        secret_access_key: Some(ROOT_SECRET_KEY.into()),
        ..AwsIamRole::default()
    })
    .unwrap()
    .with_sts_endpoint(rustfs.endpoint())
    .with_s3_endpoint(rustfs.endpoint())
    .with_client_options(ClientOptions::new().with_allow_http(true));
    (rustfs, minter)
}

fn denied(err: &object_store::Error) -> bool {
    matches!(
        err,
        object_store::Error::PermissionDenied { .. } | object_store::Error::Unauthenticated { .. }
    ) || err.to_string().contains("403")
}

#[tokio::test]
async fn minted_credentials_reach_exactly_the_requested_scope() {
    let (rustfs, minter) = setup().await;
    let request = MintRequest::new(
        format!("s3://{BUCKET}/tab").parse().unwrap(),
        AccessLevel::Read,
    )
    .with_ttl(Duration::from_secs(900))
    .with_session_name("alice@main.default.tab");
    let vended = minter.mint(&request).await.unwrap();
    assert_eq!(vended.scope, request.scope);
    assert!(vended.expires_at.is_some());
    assert_eq!(vended.hints.endpoint.as_deref(), Some(rustfs.endpoint()));

    let store = vended_store(&rustfs, &vended);
    store.get(&Path::from("tab/a")).await.unwrap();
    let err = store.get(&Path::from("table2/b")).await.unwrap_err();
    assert!(denied(&err), "read on a sibling prefix: {err}");
    let err = store
        .put(&Path::from("tab/new"), PutPayload::from_static(b"y"))
        .await
        .unwrap_err();
    assert!(denied(&err), "write with read access: {err}");
}

#[tokio::test]
async fn read_write_credentials_write_inside_only() {
    let (rustfs, minter) = setup().await;
    let request = MintRequest::new(
        format!("s3://{BUCKET}/tab").parse().unwrap(),
        AccessLevel::ReadWrite,
    );
    let store = vended_store(&rustfs, &minter.mint(&request).await.unwrap());
    store
        .put(&Path::from("tab/new"), PutPayload::from_static(b"y"))
        .await
        .unwrap();
    let err = store
        .put(&Path::from("table2/new"), PutPayload::from_static(b"y"))
        .await
        .unwrap_err();
    assert!(denied(&err), "write on a sibling prefix: {err}");
}
