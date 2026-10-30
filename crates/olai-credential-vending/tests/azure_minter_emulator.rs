//! The Azure minters against Azurite, which validates SAS signatures: a minted SAS
//! reaches its container at its access, and a user delegation key is fetched once for
//! many mints.
//!
//! Azurite has a flat namespace, so only container scopes are exercised here; directory
//! scoping is covered by the signature vectors in olai-http.
//!
//! Needs Docker; run with `cargo test -p olai-credential-vending --features emulator-it`.
#![cfg(feature = "emulator-it")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use olai_credential_vending::registered::AzureStorageKey;
use olai_credential_vending::{
    AccessLevel, AzureAccountKeySasMinter, AzureUserDelegationSasMinter, CredentialMinter,
    MintRequest, StorageCredentials, VendedCredential,
};
use olai_http::ClientOptions;
use olai_http::bearer::BearerCredential;
use olai_test_emulators::Azurite;
use olai_test_emulators::azurite::{ACCOUNT, ACCOUNT_KEY};

/// Hands out the emulator's bearer token, counting how often it is asked.
#[derive(Debug)]
struct CountingToken {
    token: String,
    calls: AtomicUsize,
}

#[async_trait]
impl olai_http::CredentialProvider for CountingToken {
    type Credential = BearerCredential;

    async fn get_credential(&self) -> olai_http::Result<Arc<BearerCredential>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(BearerCredential {
            token: self.token.clone(),
        }))
    }
}

/// The SAS as a query string.
fn query(vended: &VendedCredential) -> String {
    let StorageCredentials::AzureSas(pairs) = &vended.credentials else {
        panic!("expected a SAS");
    };
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

fn request(container: &str, access: AccessLevel) -> MintRequest {
    MintRequest::new(
        format!("abfss://{container}@{ACCOUNT}.dfs.core.windows.net")
            .parse()
            .unwrap(),
        access,
    )
}

async fn setup() -> Azurite {
    let azurite = Azurite::start().await;
    for container in ["granted", "other"] {
        azurite.create_container(container).await;
        azurite.put_blob(container, "a", b"x").await;
    }
    azurite
}

#[tokio::test]
async fn user_delegation_sas_reach_their_container_at_their_access() {
    let azurite = setup().await;
    let token = Arc::new(CountingToken {
        token: azurite.bearer_token(),
        calls: AtomicUsize::new(0),
    });
    let minter = AzureUserDelegationSasMinter::with_token_provider(token.clone())
        .with_endpoint(azurite.blob_endpoint())
        .with_client_options(ClientOptions::new().with_allow_invalid_certificates(true))
        .with_flat_namespace();

    let read = query(
        &minter
            .mint(&request("granted", AccessLevel::Read))
            .await
            .unwrap(),
    );
    assert_eq!(azurite.get_status("granted/a", &read).await, 200);
    assert_eq!(
        azurite.put_status("granted/b", &read).await,
        403,
        "write with read access"
    );
    assert_eq!(
        azurite.get_status("other/a", &read).await,
        403,
        "another container"
    );

    let write = query(
        &minter
            .mint(&request("granted", AccessLevel::ReadWrite))
            .await
            .unwrap(),
    );
    assert_eq!(azurite.put_status("granted/b", &write).await, 201);

    // Three mints, one user delegation key: the token was needed once.
    minter
        .mint(&request("other", AccessLevel::Read))
        .await
        .unwrap();
    assert_eq!(token.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn account_key_sas_reach_their_container_at_their_access() {
    let azurite = setup().await;
    let minter = AzureAccountKeySasMinter::new(&AzureStorageKey {
        account_name: ACCOUNT.into(),
        account_key: ACCOUNT_KEY.into(),
    })
    .with_endpoint(azurite.blob_endpoint())
    .with_emulator();

    let read = query(
        &minter
            .mint(&request("granted", AccessLevel::Read))
            .await
            .unwrap(),
    );
    assert_eq!(azurite.get_status("granted/a", &read).await, 200);
    assert_eq!(
        azurite.put_status("granted/b", &read).await,
        403,
        "write with read access"
    );
    assert_eq!(
        azurite.get_status("other/a", &read).await,
        403,
        "another container"
    );
}

#[tokio::test]
async fn minter_for_grants_azurite_containers_for_sub_paths() {
    let azurite = setup().await;
    let key = olai_credential_vending::RegisteredCredential::AzureStorageKey(AzureStorageKey {
        account_name: ACCOUNT.into(),
        account_key: ACCOUNT_KEY.into(),
    });
    let minter = olai_credential_vending::minter_for(&key).unwrap();

    // A request below the container: Azurite has no directory SAS, so the emulator
    // minter grants the container, signed for plain http.
    let sub_path = MintRequest::new(
        format!("abfss://granted@{ACCOUNT}.dfs.core.windows.net/tab/part")
            .parse()
            .unwrap(),
        AccessLevel::Read,
    );
    let vended = minter.mint(&sub_path).await.unwrap();
    assert!(vended.scope.contains(&sub_path.scope));
    let read = query(&vended);
    assert_eq!(azurite.get_status("granted/a", &read).await, 200);
    assert_eq!(
        azurite.get_status("other/a", &read).await,
        403,
        "another container"
    );
}
