//! SAS signatures checked by a service that validates them: Azurite, with OAuth so it
//! issues user delegation keys.
//!
//! Azurite has no hierarchical namespace, so only container SAS can be exercised here;
//! directory SAS (`sr=d`) is covered by the signature vectors in `azure::sas`.
//!
//! Needs Docker; run with `cargo test -p olai-http --features emulator-it`.
#![cfg(feature = "emulator-it")]

use olai_http::ClientOptions;
use olai_http::azure::{
    SAS_READ, UserDelegationKeyOptions, build_user_delegation_sas, fetch_user_delegation_key,
    generate_storage_key_sas, generate_user_delegation_sas_with_options,
};
use olai_test_emulators::Azurite;
use olai_test_emulators::azurite::{ACCOUNT, ACCOUNT_KEY};

const CONTAINER: &str = "vend";

async fn setup() -> Azurite {
    let azurite = Azurite::start().await;
    azurite.create_container(CONTAINER).await;
    azurite.put_blob(CONTAINER, "tab/a", b"x").await;
    azurite
}

/// Replaces the signature with a different, well-formed one.
fn tamper(sas: &str) -> String {
    sas.split('&')
        .map(|kv| match kv.strip_prefix("sig=") {
            Some(_) => "sig=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D".to_owned(),
            None => kv.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[tokio::test]
async fn user_delegation_sas_is_accepted() {
    let azurite = setup().await;
    let options = UserDelegationKeyOptions {
        endpoint: Some(azurite.blob_endpoint().to_owned()),
        client_options: ClientOptions::new().with_allow_invalid_certificates(true),
    };

    let read = generate_user_delegation_sas_with_options(
        ACCOUNT,
        CONTAINER,
        "",
        &azurite.bearer_token(),
        true,
        3600,
        &options,
    )
    .await
    .expect("fetch a user delegation key and sign");

    assert_eq!(
        azurite.get_status("vend/tab/a", &read).await,
        200,
        "get with {read}"
    );
    let list = format!("restype=container&comp=list&{read}");
    assert_eq!(azurite.get_status(CONTAINER, &list).await, 200, "list");
    // Read-only: no writes.
    assert_eq!(azurite.put_status("vend/tab/new", &read).await, 403);
    // Azurite does check the signature.
    assert_eq!(azurite.get_status("vend/tab/a", &tamper(&read)).await, 403);

    let write = generate_user_delegation_sas_with_options(
        ACCOUNT,
        CONTAINER,
        "",
        &azurite.bearer_token(),
        false,
        3600,
        &options,
    )
    .await
    .expect("sign a read-write SAS");
    assert_eq!(azurite.put_status("vend/tab/new", &write).await, 201);
}

#[tokio::test]
async fn service_sas_is_accepted() {
    let azurite = setup().await;
    let sas = generate_storage_key_sas(ACCOUNT, CONTAINER, "tab", ACCOUNT_KEY, true, 3600, true)
        .expect("sign with the account key");

    assert_eq!(
        azurite.get_status("vend/tab/a", &sas).await,
        200,
        "get with {sas}"
    );
    assert_eq!(azurite.put_status("vend/tab/new", &sas).await, 403);
    assert_eq!(azurite.get_status("vend/tab/a", &tamper(&sas)).await, 403);
}

/// One user delegation key signs SAS for several containers, so a server can cache it.
#[tokio::test]
async fn one_user_delegation_key_signs_many_sas() {
    let azurite = setup().await;
    azurite.create_container("other").await;
    azurite.put_blob("other", "x", b"y").await;
    let options = UserDelegationKeyOptions {
        endpoint: Some(azurite.blob_endpoint().to_owned()),
        client_options: ClientOptions::new().with_allow_invalid_certificates(true),
    };
    let now = chrono::Utc::now();
    let expiry = now + chrono::Duration::hours(1);
    let key = fetch_user_delegation_key(ACCOUNT, &azurite.bearer_token(), now, expiry, &options)
        .await
        .expect("fetch a user delegation key");
    assert!(key.expires_at().is_some());

    for (container, blob) in [(CONTAINER, "vend/tab/a"), ("other", "other/x")] {
        let sas = build_user_delegation_sas(ACCOUNT, container, "", &key, expiry, SAS_READ)
            .expect("sign");
        assert_eq!(
            azurite.get_status(blob, &sas).await,
            200,
            "get {blob} with {sas}"
        );
    }
}
