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
    let creds = scoped_credentials(rustfs, scope, access).await;
    store(
        rustfs,
        &creds.key_id,
        &creds.secret_key,
        creds.token.as_deref(),
    )
}

/// Session credentials minted for `scope` at `access`.
async fn scoped_credentials(rustfs: &RustFs, scope: &str, access: AccessLevel) -> AwsCredential {
    let scope: StorageScope = scope.parse().unwrap();
    mint(rustfs, session_policy(&scope, access).unwrap()).await
}

/// Session credentials narrowed by `policy`.
async fn mint(rustfs: &RustFs, policy: String) -> AwsCredential {
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
    AwsCredential {
        key_id: creds.key_id.clone(),
        secret_key: creds.secret_key.clone(),
        token: creds.token.clone(),
    }
}

/// `ListObjectsV2` with exactly `prefix`, signed by hand (SigV4), returning the HTTP
/// status and body.
async fn raw_list(rustfs: &RustFs, creds: &AwsCredential, prefix: &str) -> (u16, String) {
    use ring::{digest, hmac};

    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let sha256 = |data: &[u8]| hex(digest::digest(&digest::SHA256, data).as_ref());
    let mac = |key: &[u8], data: &str| {
        hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes())
            .as_ref()
            .to_vec()
    };
    // SigV4 query encoding: everything but unreserved characters.
    let encode = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };

    let endpoint = url::Url::parse(rustfs.endpoint()).unwrap();
    let host = &endpoint[url::Position::BeforeHost..url::Position::AfterPort];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let now = chrono::DateTime::from_timestamp(now.as_secs() as i64, 0).unwrap();
    let date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let day = &date[..8];
    let token = creds.token.as_deref().unwrap();
    let payload = sha256(b"");

    let path = format!("/{BUCKET}");
    let query = format!("list-type=2&prefix={}", encode(prefix));
    let signed_headers = "host;x-amz-content-sha256;x-amz-date;x-amz-security-token";
    let canonical = format!(
        "GET\n{path}\n{query}\nhost:{host}\nx-amz-content-sha256:{payload}\n\
         x-amz-date:{date}\nx-amz-security-token:{token}\n\n{signed_headers}\n{payload}"
    );
    let scope = format!("{day}/{REGION}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        sha256(canonical.as_bytes())
    );
    let key = [day, REGION, "s3", "aws4_request"].iter().fold(
        format!("AWS4{}", creds.secret_key).into_bytes(),
        |key, part| mac(&key, part),
    );
    let signature = hex(&mac(&key, &string_to_sign));

    let response = reqwest::Client::new()
        .get(format!("{}{path}?{query}", rustfs.endpoint().trim_end_matches('/')))
        .header("x-amz-content-sha256", &payload)
        .header("x-amz-date", &date)
        .header("x-amz-security-token", token)
        .header(
            "authorization",
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                creds.key_id
            ),
        )
        .send()
        .await
        .expect("send ListObjectsV2");
    let status = response.status().as_u16();
    (status, response.text().await.unwrap_or_default())
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
async fn bare_prefix_listing_cannot_reveal_sibling_keys() {
    let rustfs = setup().await;
    let creds = scoped_credentials(&rustfs, "s3://vend/tab", AccessLevel::Read).await;

    let (status, body) = raw_list(&rustfs, &creds, "tab/").await;
    assert_eq!(status, 200, "LIST tab/: {body}");
    assert!(body.contains("tab/a") && !body.contains("table2"), "{body}");

    // `prefix=tab` would match `table2/c` as a string.
    let (status, body) = raw_list(&rustfs, &creds, "tab").await;
    assert_eq!(status, 403, "LIST tab must be denied: {body}");

    // Mutation check: with the bare prefix allowed, the same request leaks `table2`, so
    // the denial above is the policy's doing.
    let weakened = session_policy(&"s3://vend/tab".parse().unwrap(), AccessLevel::Read)
        .unwrap()
        .replace(r#""tab/","tab/*""#, r#""tab","tab/*""#);
    assert!(weakened.contains(r#""tab","tab/*""#), "{weakened}");
    let weak = mint(&rustfs, weakened).await;
    let (status, body) = raw_list(&rustfs, &weak, "tab").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("table2/c"), "{body}");
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
