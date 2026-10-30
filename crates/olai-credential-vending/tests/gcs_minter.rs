//! GCS server and client, end to end, against a mock of Google's token APIs and GCS.
//!
//! No emulator enforces Credential Access Boundaries, so this checks the plumbing (the
//! token the server mints is the one the client's store sends) and the gated live test
//! (`gcs_minter_live`) checks enforcement. The path is the one a Unity Catalog server
//! and client would run:
//!
//! 1. the server resolves the service account governing the location and mints
//!    through it ([`Vendor`]);
//! 2. it serializes the token as a UC REST `temporary-*-credentials` response;
//! 3. the client parses the response for its request and builds a store from it
//!    ([`build_store`]).

use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use mockito::Matcher;
use object_store::ObjectStoreExt;
use object_store::path::Path;
use olai_credential_vending::registered::GcpServiceAccount;
use olai_credential_vending::{
    AccessLevel, CredentialMinter, ExternalLocations, GcsDownscopedMinter, MintRequest,
    RegisteredCredential, Vendor,
};
use olai_http::gcp::GcpCredential;
use olai_http::{ClientOptions, StaticCredentialProvider};
use olai_storage_credentials::wire::VendContext;
use olai_storage_credentials::wire::unity::TemporaryCredential;
use olai_vended_store::{CredentialSource, StoreOptions, VendedCredential, build_store};
use serde_json::json;

const EMAIL: &str = "vend@project.iam.gserviceaccount.com";

#[derive(Debug)]
struct Endpoint {
    vendor: Vendor<ExternalLocations>,
    request: MintRequest,
}

#[async_trait::async_trait]
impl CredentialSource for Endpoint {
    async fn fetch(&self) -> olai_vended_store::Result<VendedCredential> {
        let minted = self
            .vendor
            .vend(&self.request)
            .await
            .map_err(olai_vended_store::Error::vend)?;
        let body = serde_json::to_string(&TemporaryCredential::from(&minted)).unwrap();
        let wire: TemporaryCredential = serde_json::from_str(&body).unwrap();
        Ok(wire.to_vended(&VendContext::new(
            self.request.scope.clone(),
            self.request.access,
        ))?)
    }
}

#[tokio::test]
async fn the_store_sends_the_downscoped_token() {
    let mut server = mockito::Server::new_async().await;
    let url = server.url();
    let iam = server
        .mock(
            "POST",
            format!("/v1/projects/-/serviceAccounts/{EMAIL}:generateAccessToken").as_str(),
        )
        .match_header("authorization", "Bearer host-token")
        .with_body(
            json!({
                "accessToken": "sa-token",
                "expireTime": (Utc::now() + TimeDelta::hours(1)).to_rfc3339(),
            })
            .to_string(),
        )
        .create_async()
        .await;
    let sts = server
        .mock("POST", "/v1/token")
        .match_body(Matcher::UrlEncoded(
            "subject_token".into(),
            "sa-token".into(),
        ))
        .with_body(r#"{"access_token":"downscoped-token","expires_in":3600}"#)
        .create_async()
        .await;
    let object = server
        // GCS percent-encodes the object name into one segment.
        .mock(
            "GET",
            Matcher::Regex("^/bucket/warehouse%2Ftab%2Fpart%2D0%2Eparquet$".into()),
        )
        .match_header("authorization", "Bearer downscoped-token")
        .with_header("etag", "\"1\"")
        .with_header("last-modified", "Tue, 01 Jan 2030 00:00:00 GMT")
        .with_body("data")
        .create_async()
        .await;

    let locations = ExternalLocations::new().with_location(
        "gs://bucket/warehouse".parse().unwrap(),
        RegisteredCredential::GcpServiceAccount(GcpServiceAccount {
            email: EMAIL.into(),
            key: None,
        }),
    );
    let vendor = Vendor::new(locations).with_minters(move |credential| {
        let RegisteredCredential::GcpServiceAccount(account) = credential else {
            panic!("only a service account is registered");
        };
        let minter: Arc<dyn CredentialMinter> = Arc::new(
            GcsDownscopedMinter::new(account)?
                .with_base_credentials(Arc::new(StaticCredentialProvider::new(GcpCredential {
                    bearer: "host-token".into(),
                })))
                .with_iam_credentials_endpoint(&url)
                .with_sts_endpoint(format!("{url}/v1/token"))
                .with_client_options(ClientOptions::new().with_allow_http(true)),
        );
        Ok(minter)
    });
    let source = Arc::new(Endpoint {
        vendor,
        request: MintRequest::new(
            "gs://bucket/warehouse/tab".parse().unwrap(),
            AccessLevel::Read,
        ),
    });

    let mut seed = source.fetch().await.unwrap();
    // UC REST carries no endpoint; a client takes it from configuration.
    seed.hints.endpoint = Some(server.url());
    seed.hints.allow_http = Some(true);
    let store = build_store(&seed, source, &StoreOptions::default())
        .await
        .unwrap();
    let data = store
        .prefixed()
        .get(&Path::from("part-0.parquet"))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(data.as_ref(), b"data");

    iam.assert_async().await;
    sts.assert_async().await;
    object.assert_async().await;
}
