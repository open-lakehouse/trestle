//! GCS downscoping against real Google Cloud: the grant a Credential Access Boundary
//! enforces, which no emulator models.
//!
//! Opt-in. It needs a bucket the test may write under `p4-live/`, and a service account
//! with `roles/storage.objectAdmin` on it. Without a key, the caller's application
//! default credentials impersonate the account and need
//! `roles/iam.serviceAccountTokenCreator` on it:
//!
//! ```bash
//! gcloud auth application-default login
//! OLAI_GCS_LIVE_BUCKET=my-bucket \
//! OLAI_GCS_LIVE_SERVICE_ACCOUNT=vend@my-project.iam.gserviceaccount.com \
//!   cargo test -p olai-credential-vending --features gcp-live --test gcs_minter_live
//! ```
//!
//! Set `OLAI_GCS_LIVE_KEY_FILE` to a key of the account to mint from the key instead.
//! Without the variables the test passes without calling Google.
#![cfg(feature = "gcp-live")]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use olai_credential_vending::registered::{GcpServiceAccount, GcpServiceAccountKey};
use olai_credential_vending::{
    AccessLevel, CredentialMinter, GcsDownscopedMinter, MintRequest, StorageCredentials,
};
use olai_http::bearer::GcpServiceAccount as KeySource;
use olai_http::gcp::{
    CLOUD_PLATFORM_SCOPE, GcpTokenOptions, GoogleBuilder, impersonate_access_token,
    service_account_access_token,
};
use olai_vended_store::{CredentialSource, StoreOptions, VendedCredential, build_store};

struct Live {
    bucket: String,
    account: GcpServiceAccount,
    /// This run's prefix, below which everything is written and then deleted.
    run: String,
}

fn live() -> Option<Live> {
    let (Ok(bucket), Ok(email)) = (
        std::env::var("OLAI_GCS_LIVE_BUCKET"),
        std::env::var("OLAI_GCS_LIVE_SERVICE_ACCOUNT"),
    ) else {
        eprintln!("OLAI_GCS_LIVE_BUCKET / OLAI_GCS_LIVE_SERVICE_ACCOUNT unset; skipping");
        return None;
    };
    let key = std::env::var("OLAI_GCS_LIVE_KEY_FILE")
        .ok()
        .map(GcpServiceAccountKey::ServiceAccountKeyFile);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Some(Live {
        bucket,
        account: GcpServiceAccount { email, key },
        run: format!("p4-live/{nanos}"),
    })
}

/// Re-vends the same request, as a vending endpoint would.
#[derive(Debug)]
struct Mint {
    minter: Arc<GcsDownscopedMinter>,
    request: MintRequest,
}

#[async_trait]
impl CredentialSource for Mint {
    async fn fetch(&self) -> olai_vended_store::Result<VendedCredential> {
        self.minter
            .mint(&self.request)
            .await
            .map_err(olai_vended_store::Error::vend)
    }
}

/// Serves one fixed credential.
#[derive(Debug)]
struct Fixed(VendedCredential);

#[async_trait]
impl CredentialSource for Fixed {
    async fn fetch(&self) -> olai_vended_store::Result<VendedCredential> {
        Ok(self.0.clone())
    }
}

/// A store over the whole bucket with the account's own, undownscoped token, for
/// fixtures and clean-up.
async fn admin(live: &Live) -> Arc<dyn ObjectStore> {
    let options = GcpTokenOptions::default();
    let token = match &live.account.key {
        Some(GcpServiceAccountKey::ServiceAccountKeyFile(path)) => service_account_access_token(
            &KeySource::File(path.clone()),
            CLOUD_PLATFORM_SCOPE,
            &options,
        )
        .await
        .unwrap(),
        _ => {
            let base = GoogleBuilder::from_env().build(None).unwrap().credentials;
            impersonate_access_token(
                &base,
                &live.account.email,
                Duration::from_secs(900),
                &[CLOUD_PLATFORM_SCOPE],
                &options,
            )
            .await
            .unwrap()
        }
    };
    let credential = VendedCredential::new(
        StorageCredentials::GcpBearer(token.credential.bearer),
        format!("gs://{}", live.bucket).parse().unwrap(),
        AccessLevel::ReadWrite,
    )
    .with_expiry(Some(token.expiration));
    build_store(
        &credential,
        Arc::new(Fixed(credential.clone())),
        &StoreOptions::default(),
    )
    .await
    .unwrap()
    .root()
}

/// A bucket-rooted store on a downscoped grant for `table` under the run.
async fn vended(live: &Live, table: &str, access: AccessLevel) -> Arc<dyn ObjectStore> {
    let minter = Arc::new(GcsDownscopedMinter::new(&live.account).unwrap());
    let request = MintRequest::new(
        format!("gs://{}/{}/{table}", live.bucket, live.run)
            .parse()
            .unwrap(),
        access,
    )
    .with_ttl(Duration::from_secs(900));
    let seed = minter.mint(&request).await.unwrap();
    assert!(seed.expires_at.is_some());
    build_store(
        &seed,
        Arc::new(Mint { minter, request }),
        &StoreOptions::default(),
    )
    .await
    .unwrap()
    .root()
}

fn denied(err: &object_store::Error) -> bool {
    matches!(
        err,
        object_store::Error::PermissionDenied { .. } | object_store::Error::Unauthenticated { .. }
    ) || err.to_string().contains("403")
}

#[tokio::test]
async fn boundaries_are_enforced() {
    let Some(live) = live() else { return };
    let admin = admin(&live).await;
    let path = |key: &str| Path::from(format!("{}/{key}", live.run));
    for key in ["tab/a", "tab2/b"] {
        admin
            .put(&path(key), PutPayload::from_static(b"x"))
            .await
            .unwrap();
    }

    // Read: inside the table only, and no writes.
    let read = vended(&live, "tab", AccessLevel::Read).await;
    read.get(&path("tab/a")).await.unwrap();
    let listed = read.list_with_delimiter(Some(&path("tab"))).await.unwrap();
    assert_eq!(listed.objects.len(), 1);
    let err = read.get(&path("tab2/b")).await.unwrap_err();
    assert!(denied(&err), "read on a sibling prefix: {err}");
    let err = read.list_with_delimiter(None).await.unwrap_err();
    assert!(denied(&err), "list of the bucket root: {err}");
    let err = read
        .put(&path("tab/new"), PutPayload::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(denied(&err), "write on a read grant: {err}");

    // Read-write: writes inside the table, never beside it.
    let write = vended(&live, "tab", AccessLevel::ReadWrite).await;
    write
        .put(&path("tab/new"), PutPayload::from_static(b"x"))
        .await
        .unwrap();
    write.delete(&path("tab/new")).await.unwrap();
    let err = write
        .put(&path("tab2/new"), PutPayload::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(denied(&err), "write on a sibling prefix: {err}");

    for key in ["tab/a", "tab2/b"] {
        admin.delete(&path(key)).await.unwrap();
    }
}
