//! Azurite: the Azure Blob Storage emulator, with OAuth so it issues and validates
//! user-delegation keys.
//!
//! [`Azurite::start`] runs the blob service over HTTPS (a self-signed certificate
//! generated per run) with `--oauth basic`. In that mode Azurite accepts any well-formed
//! bearer JWT with a storage audience and a known issuer, without checking its
//! signature, so [`Azurite::bearer_token`] can mint one locally. It still validates
//! shared key and SAS signatures strictly, including user-delegation SAS signed with a
//! key from its `Get User Delegation Key` endpoint.
//!
//! Azurite has a flat namespace: directory SAS (`sr=d`, `sdd`) is not supported and
//! always fails authorization.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// Azurite image tag.
pub const AZURITE_TAG: &str = "3.37.0";

/// The emulator's built-in storage account.
pub const ACCOUNT: &str = "devstoreaccount1";

/// The built-in account's well-known key (public; documented by Microsoft).
pub const ACCOUNT_KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

/// Tenant id in the bearer tokens [`Azurite::bearer_token`] mints.
pub const TENANT_ID: &str = "ab1f708d-50f6-404c-a006-d71b2ac7a606";

/// Object id in the bearer tokens [`Azurite::bearer_token`] mints.
pub const OBJECT_ID: &str = "23657296-5cd5-45b0-a809-d972a7f4dfe1";

/// The Azure Storage API version requests use.
const API_VERSION: &str = "2020-12-06";

const PORT: u16 = 10000;

/// A running Azurite blob service with OAuth over HTTPS. Stopped and removed on drop.
pub struct Azurite {
    // Held so the container lives as long as this value.
    _container: ContainerAsync<GenericImage>,
    blob_endpoint: String,
    http: reqwest::Client,
}

impl Azurite {
    /// Starts Azurite and waits until the blob service listens.
    ///
    /// # Panics
    ///
    /// If Docker is unavailable or the container does not start.
    pub async fn start() -> Self {
        let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into(), "localhost".into()])
            .expect("generate a self-signed certificate");
        let container = GenericImage::new("mcr.microsoft.com/azure-storage/azurite", AZURITE_TAG)
            .with_exposed_port(PORT.tcp())
            .with_wait_for(WaitFor::message_on_stdout("successfully listens on"))
            .with_copy_to("/certs/cert.pem", cert.cert.pem().into_bytes())
            .with_copy_to(
                "/certs/key.pem",
                cert.signing_key.serialize_pem().into_bytes(),
            )
            .with_cmd([
                "azurite-blob",
                "--blobHost",
                "0.0.0.0",
                "--oauth",
                "basic",
                "--cert",
                "/certs/cert.pem",
                "--key",
                "/certs/key.pem",
                "--skipApiVersionCheck",
            ])
            .start()
            .await
            .expect("start Azurite (is Docker running?)");
        let port = container
            .get_host_port_ipv4(PORT.tcp())
            .await
            .expect("Azurite port");
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("build HTTP client");
        Self {
            _container: container,
            blob_endpoint: format!("https://127.0.0.1:{port}/{ACCOUNT}"),
            http,
        }
    }

    /// The blob endpoint including the account path
    /// (`https://127.0.0.1:<port>/devstoreaccount1`). Its certificate is self-signed.
    pub fn blob_endpoint(&self) -> &str {
        &self.blob_endpoint
    }

    /// A bearer token Azurite's basic OAuth accepts, valid for an hour.
    pub fn bearer_token(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs();
        let header = r#"{"alg":"RS256","typ":"JWT"}"#;
        let claims = format!(
            r#"{{"aud":"https://storage.azure.com","iss":"https://sts.windows.net/{TENANT_ID}/","nbf":{nbf},"iat":{nbf},"exp":{exp},"oid":"{OBJECT_ID}","tid":"{TENANT_ID}"}}"#,
            nbf = now - 60,
            exp = now + 3600,
        );
        format!(
            "{}.{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(claims),
            // Not checked in basic mode.
            URL_SAFE_NO_PAD.encode("unsigned"),
        )
    }

    /// Creates a container, authenticating with [`bearer_token`](Self::bearer_token).
    ///
    /// # Panics
    ///
    /// If the request fails.
    pub async fn create_container(&self, container: &str) {
        self.authorized(
            self.http.put(format!(
                "{}/{container}?restype=container",
                self.blob_endpoint
            )),
            "create container",
        )
        .await;
    }

    /// Uploads a block blob, authenticating with [`bearer_token`](Self::bearer_token).
    ///
    /// # Panics
    ///
    /// If the request fails.
    pub async fn put_blob(&self, container: &str, name: &str, body: &'static [u8]) {
        self.authorized(
            self.http
                .put(format!("{}/{container}/{name}", self.blob_endpoint))
                .header("x-ms-blob-type", "BlockBlob")
                .body(body),
            "put blob",
        )
        .await;
    }

    /// GETs `path` (relative to the account) with `query` appended, unauthenticated
    /// apart from what the query carries (a SAS), and returns the HTTP status.
    pub async fn get_status(&self, path: &str, query: &str) -> u16 {
        self.http
            .get(format!("{}/{path}?{query}", self.blob_endpoint))
            .header("x-ms-version", API_VERSION)
            .send()
            .await
            .expect("send request to Azurite")
            .status()
            .as_u16()
    }

    /// PUTs a block blob at `path` (relative to the account) with `query` appended,
    /// authorized only by what the query carries (a SAS), and returns the HTTP status.
    pub async fn put_status(&self, path: &str, query: &str) -> u16 {
        self.http
            .put(format!("{}/{path}?{query}", self.blob_endpoint))
            .header("x-ms-version", API_VERSION)
            .header("x-ms-blob-type", "BlockBlob")
            .body("y")
            .send()
            .await
            .expect("send request to Azurite")
            .status()
            .as_u16()
    }

    async fn authorized(&self, request: reqwest::RequestBuilder, what: &str) {
        let response = request
            .bearer_auth(self.bearer_token())
            .header("x-ms-version", API_VERSION)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            panic!("{what} failed ({status}): {body}");
        }
    }
}
