//! One-off GCP token calls with wall-clock expiries.
//!
//! The [`GcpCredentialProvider`]s of [`GoogleBuilder`](super::GoogleBuilder) cache
//! their tokens and hide when they expire. A credential vending server needs that
//! expiry: it hands tokens to clients, and a downscoped token cannot outlive the token
//! it was exchanged from. These functions make one call each and return a
//! [`GcpToken`] with the expiry Google reports:
//!
//! - [`impersonate_access_token`]: an access token for a service account, requested
//!   by another identity (`generateAccessToken`), for a chosen lifetime.
//! - [`service_account_access_token`]: an OAuth access token for a service-account
//!   key (the JWT-bearer grant). Unlike the self-signed JWTs `GoogleBuilder` signs
//!   with a key, these can be exchanged at STS.
//! - [`downscope_token`]: an access token exchanged at STS for one restricted by a
//!   Credential Access Boundary.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::{Client, Method};
use serde::Serialize;

use super::GcpCredentialProvider;
use super::credential::{
    DEFAULT_SCOPE, GcpCredential, ImpersonateTokenResponse, TokenResponse, seconds_since_epoch,
    sign_jwt,
};
use crate::bearer::{
    ACCESS_TOKEN_TYPE, GcpServiceAccount, JWT_BEARER_GRANT, LoadedServiceAccount,
    TOKEN_EXCHANGE_GRANT,
};
use crate::retry::RetryExt;
use crate::service::{HttpService, make_service};
use crate::{ClientOptions, Error, Result, RetryConfig};

/// The OAuth scope of all Google Cloud APIs.
pub const CLOUD_PLATFORM_SCOPE: &str = DEFAULT_SCOPE;

/// Google's IAM Service Account Credentials API.
pub const IAM_CREDENTIALS_ENDPOINT: &str = "https://iamcredentials.googleapis.com";

/// Google's Security Token Service token endpoint.
pub const STS_TOKEN_ENDPOINT: &str = "https://sts.googleapis.com/v1/token";

/// The lifetime of an access token from a service-account key; Google's token
/// endpoint issues no other.
const SERVICE_ACCOUNT_TOKEN_LIFETIME: u64 = 3600;

/// An access token and when it expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcpToken {
    /// The token (`Debug` redacts it).
    pub credential: GcpCredential,
    /// When Google says the token expires.
    pub expiration: DateTime<Utc>,
}

/// How to make one of this module's calls.
#[derive(Debug, Clone, Default)]
pub struct GcpTokenOptions {
    /// Overrides the endpoint called: the base URL of the IAM Service Account
    /// Credentials API ([`IAM_CREDENTIALS_ENDPOINT`]), the key's token URI, or the STS
    /// token URL ([`STS_TOKEN_ENDPOINT`]).
    pub endpoint: Option<String>,
    /// HTTP client options.
    pub client_options: ClientOptions,
    /// Retries.
    pub retry: RetryConfig,
}

/// An access token for the service account `email`, requested with `base`'s
/// credentials, for `lifetime` (whole seconds; Google allows up to one hour, or twelve
/// where the organization policy extends it).
///
/// `base` must hold `iam.serviceAccounts.getAccessToken` on the account, for example
/// through `roles/iam.serviceAccountTokenCreator`.
///
/// <https://cloud.google.com/iam/docs/reference/credentials/rest/v1/projects.serviceAccounts/generateAccessToken>
pub async fn impersonate_access_token(
    base: &GcpCredentialProvider,
    email: &str,
    lifetime: Duration,
    scopes: &[&str],
    options: &GcpTokenOptions,
) -> Result<GcpToken> {
    if email.is_empty() || email.contains(['/', '?', '#', '%']) {
        return Err(generic(format!("invalid service account `{email}`")));
    }
    let endpoint = options
        .endpoint
        .as_deref()
        .unwrap_or(IAM_CREDENTIALS_ENDPOINT)
        .trim_end_matches('/');
    let url = format!("{endpoint}/v1/projects/-/serviceAccounts/{email}:generateAccessToken");
    let base = base.get_credential().await?;
    let client = options.client_options.client()?;
    let service = make_service(client.clone(), None);
    generate_access_token(
        &client,
        &service,
        &options.retry,
        &url,
        &base.bearer,
        lifetime,
        scopes,
    )
    .await
}

/// One `generateAccessToken` call to `url` (the full resource URL of the service
/// account), authenticated with `bearer`. Shared by [`impersonate_access_token`] and
/// the impersonation step of the WIF provider, so both fail the same way on a bad
/// `expireTime`.
pub(crate) async fn generate_access_token(
    client: &Client,
    service: &Arc<dyn HttpService>,
    retry: &RetryConfig,
    url: &str,
    bearer: &str,
    lifetime: Duration,
    scopes: &[&str],
) -> Result<GcpToken> {
    #[derive(Serialize)]
    struct Request<'a> {
        scope: &'a [&'a str],
        lifetime: String,
    }
    let response: ImpersonateTokenResponse = client
        .request(Method::POST, url)
        .bearer_auth(bearer)
        .json(&Request {
            scope: scopes,
            lifetime: format!("{}s", lifetime.as_secs()),
        })
        .retryable(retry, service.clone())
        .idempotent(true)
        .send()
        .await
        .map_err(|e| e.error())?
        .json()
        .await?;
    let expiration = DateTime::parse_from_rfc3339(&response.expire_time)
        .map_err(|e| {
            generic(format!(
                "invalid `expireTime` `{}`: {e}",
                response.expire_time
            ))
        })?
        .with_timezone(&Utc);
    Ok(GcpToken {
        credential: GcpCredential {
            bearer: response.access_token,
        },
        expiration,
    })
}

/// An OAuth access token for `account`, with `scope`, through the JWT-bearer grant at
/// the key's token URI. It lives one hour.
///
/// <https://developers.google.com/identity/protocols/oauth2/service-account#httprest>
pub async fn service_account_access_token(
    account: &GcpServiceAccount,
    scope: &str,
    options: &GcpTokenOptions,
) -> Result<GcpToken> {
    let LoadedServiceAccount {
        credentials,
        key,
        token_uri,
    } = account.load("access tokens")?;
    let token_uri = options.endpoint.clone().unwrap_or(token_uri);

    #[derive(Serialize)]
    struct Claims<'a> {
        iss: &'a str,
        scope: &'a str,
        aud: &'a str,
        iat: u64,
        exp: u64,
    }
    let iat = seconds_since_epoch();
    let assertion = sign_jwt(
        &key,
        &credentials.private_key_id,
        &Claims {
            iss: &credentials.client_email,
            scope,
            aud: &token_uri,
            iat,
            exp: iat + SERVICE_ACCOUNT_TOKEN_LIFETIME,
        },
    )?;
    let issued = Utc::now();
    let client = options.client_options.client()?;
    let service = make_service(client.clone(), None);
    let response: TokenResponse = client
        .request(Method::POST, &token_uri)
        .form(&[("grant_type", JWT_BEARER_GRANT), ("assertion", &assertion)])
        .retryable(&options.retry, service)
        .idempotent(true)
        .send()
        .await
        .map_err(|e| e.error())?
        .json()
        .await?;
    Ok(GcpToken {
        credential: GcpCredential {
            bearer: response.access_token,
        },
        expiration: after(issued, response.expires_in),
    })
}

/// Exchanges `source` at STS for a token restricted by `access_boundary`, the
/// Credential Access Boundary document (`{"accessBoundary": {...}}`).
///
/// The downscoped token cannot outlive `source`: its expiry is the one STS reports,
/// or `source`'s when STS reports none, and never later than `source`'s.
///
/// <https://cloud.google.com/iam/docs/downscoping-short-lived-credentials>
pub async fn downscope_token(
    source: &GcpToken,
    access_boundary: &serde_json::Value,
    options: &GcpTokenOptions,
) -> Result<GcpToken> {
    #[derive(serde::Deserialize)]
    struct Response {
        access_token: String,
        expires_in: Option<u64>,
    }

    let endpoint = options.endpoint.as_deref().unwrap_or(STS_TOKEN_ENDPOINT);
    let boundary = access_boundary.to_string();
    let issued = Utc::now();
    let client = options.client_options.client()?;
    let service = make_service(client.clone(), None);
    let response: Response = client
        .request(Method::POST, endpoint)
        .form(&[
            ("grant_type", TOKEN_EXCHANGE_GRANT),
            ("subject_token_type", ACCESS_TOKEN_TYPE),
            ("requested_token_type", ACCESS_TOKEN_TYPE),
            ("subject_token", &source.credential.bearer),
            ("options", &boundary),
        ])
        .retryable(&options.retry, service)
        .idempotent(true)
        .send()
        .await
        .map_err(|e| e.error())?
        .json()
        .await?;
    let expiration = response
        .expires_in
        .map_or(source.expiration, |secs| after(issued, secs))
        .min(source.expiration);
    Ok(GcpToken {
        credential: GcpCredential {
            bearer: response.access_token,
        },
        expiration,
    })
}

/// `secs` after `issued`, saturating far in the future.
fn after(issued: DateTime<Utc>, secs: u64) -> DateTime<Utc> {
    i64::try_from(secs)
        .ok()
        .and_then(TimeDelta::try_seconds)
        .and_then(|delta| issued.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

fn generic(message: String) -> Error {
    Error::Generic {
        source: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use base64::Engine;
    use base64::prelude::BASE64_URL_SAFE_NO_PAD;
    use mockito::Matcher;
    use serde_json::json;

    use super::*;
    use crate::StaticCredentialProvider;

    fn options(server: &mockito::Server, path: &str) -> GcpTokenOptions {
        GcpTokenOptions {
            endpoint: Some(format!("{}{path}", server.url())),
            client_options: ClientOptions::new().with_allow_http(true),
            retry: RetryConfig {
                max_retries: 0,
                ..RetryConfig::default()
            },
        }
    }

    fn token(bearer: &str, expiration: DateTime<Utc>) -> GcpToken {
        GcpToken {
            credential: GcpCredential {
                bearer: bearer.into(),
            },
            expiration,
        }
    }

    fn base() -> GcpCredentialProvider {
        Arc::new(StaticCredentialProvider::new(GcpCredential {
            bearer: "base-token".into(),
        }))
    }

    /// A service-account key file around a freshly generated RSA key: no key material
    /// is checked in.
    fn service_account(token_uri: &str) -> GcpServiceAccount {
        static PEM: OnceLock<String> = OnceLock::new();
        let pem = PEM.get_or_init(|| {
            use rsa::pkcs8::{EncodePrivateKey, LineEnding};
            let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap();
            key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string()
        });
        GcpServiceAccount::Key(
            json!({
                "type": "service_account",
                "private_key": pem,
                "private_key_id": "kid",
                "client_email": "vend@project.iam.gserviceaccount.com",
                "token_uri": token_uri,
            })
            .to_string(),
        )
    }

    fn jwt_claims(jwt: &str) -> serde_json::Value {
        let payload = jwt.split('.').nth(1).unwrap();
        serde_json::from_slice(&BASE64_URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn impersonation_requests_the_lifetime() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "POST",
                "/v1/projects/-/serviceAccounts/vend@project.iam.gserviceaccount.com:generateAccessToken",
            )
            .match_header("authorization", "Bearer base-token")
            .match_body(Matcher::Json(json!({
                "scope": [CLOUD_PLATFORM_SCOPE],
                "lifetime": "1800s",
            })))
            .with_body(r#"{"accessToken":"sa-token","expireTime":"2030-01-01T00:30:00Z"}"#)
            .create_async()
            .await;
        let token = impersonate_access_token(
            &base(),
            "vend@project.iam.gserviceaccount.com",
            Duration::from_secs(1800),
            &[CLOUD_PLATFORM_SCOPE],
            &options(&server, ""),
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(token.credential.bearer, "sa-token");
        assert_eq!(token.expiration.to_rfc3339(), "2030-01-01T00:30:00+00:00");
    }

    #[tokio::test]
    async fn impersonation_rejects_bad_expiries_and_accounts() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", Matcher::Any)
            .with_body(r#"{"accessToken":"sa-token","expireTime":"soon"}"#)
            .create_async()
            .await;
        let err = impersonate_access_token(
            &base(),
            "vend@project.iam.gserviceaccount.com",
            Duration::from_secs(600),
            &[CLOUD_PLATFORM_SCOPE],
            &options(&server, ""),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("expireTime"), "{err}");
        assert!(!err.to_string().contains("sa-token"), "{err}");

        for email in ["", "a/b@x", "a@x?y", "a%2F@x"] {
            let err = impersonate_access_token(
                &base(),
                email,
                Duration::from_secs(600),
                &[CLOUD_PLATFORM_SCOPE],
                &options(&server, ""),
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("invalid service account"), "{err}");
        }
    }

    #[tokio::test]
    async fn service_account_key_uses_the_jwt_bearer_grant() {
        let mut server = mockito::Server::new_async().await;
        let token_uri = format!("{}/token", server.url());
        let mock = server
            .mock("POST", "/token")
            .match_body(Matcher::UrlEncoded(
                "grant_type".into(),
                JWT_BEARER_GRANT.into(),
            ))
            .with_body(r#"{"access_token":"oauth-token","expires_in":3599,"token_type":"Bearer"}"#)
            .create_async()
            .await;
        let before = Utc::now();
        let token = service_account_access_token(
            &service_account(&token_uri),
            CLOUD_PLATFORM_SCOPE,
            &GcpTokenOptions {
                client_options: ClientOptions::new().with_allow_http(true),
                ..GcpTokenOptions::default()
            },
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(token.credential.bearer, "oauth-token");
        assert!(token.expiration >= before + TimeDelta::seconds(3599));
        assert!(token.expiration <= Utc::now() + TimeDelta::seconds(3599));
    }

    #[tokio::test]
    async fn service_account_assertion_claims() {
        let mut server = mockito::Server::new_async().await;
        let token_uri = format!("{}/token", server.url());
        let captured = Arc::new(std::sync::Mutex::new(String::new()));
        let sink = Arc::clone(&captured);
        let _mock = server
            .mock("POST", "/token")
            .with_body_from_request(move |request| {
                *sink.lock().unwrap() = String::from_utf8(request.body().unwrap().clone()).unwrap();
                br#"{"access_token":"oauth-token","expires_in":3600}"#.to_vec()
            })
            .create_async()
            .await;
        service_account_access_token(
            &service_account(&token_uri),
            CLOUD_PLATFORM_SCOPE,
            &GcpTokenOptions {
                client_options: ClientOptions::new().with_allow_http(true),
                ..GcpTokenOptions::default()
            },
        )
        .await
        .unwrap();
        let body = captured.lock().unwrap().clone();
        let assertion = url::form_urlencoded::parse(body.as_bytes())
            .find(|(k, _)| k == "assertion")
            .unwrap()
            .1
            .into_owned();
        let claims = jwt_claims(&assertion);
        assert_eq!(claims["iss"], "vend@project.iam.gserviceaccount.com");
        assert_eq!(claims["aud"], token_uri);
        assert_eq!(claims["scope"], CLOUD_PLATFORM_SCOPE);
        assert_eq!(
            claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
            SERVICE_ACCOUNT_TOKEN_LIFETIME
        );
    }

    #[tokio::test]
    async fn service_account_rejects_user_credentials() {
        let key = r#"{"type":"authorized_user","client_id":"id","client_secret":"secret","refresh_token":"refresh"}"#;
        let err = service_account_access_token(
            &GcpServiceAccount::Key(key.into()),
            CLOUD_PLATFORM_SCOPE,
            &GcpTokenOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("`authorized_user`"), "{err}");
        assert!(!err.to_string().contains("secret"), "{err}");
    }

    #[tokio::test]
    async fn downscope_exchanges_with_the_boundary() {
        let boundary = json!({ "accessBoundary": { "accessBoundaryRules": [{
            "availableResource": "//storage.googleapis.com/projects/_/buckets/b",
            "availablePermissions": ["inRole:roles/storage.objectViewer"],
        }]}});
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), TOKEN_EXCHANGE_GRANT.into()),
                Matcher::UrlEncoded("subject_token_type".into(), ACCESS_TOKEN_TYPE.into()),
                Matcher::UrlEncoded("requested_token_type".into(), ACCESS_TOKEN_TYPE.into()),
                Matcher::UrlEncoded("subject_token".into(), "source-token".into()),
                Matcher::UrlEncoded("options".into(), boundary.to_string()),
            ]))
            .with_body(r#"{"access_token":"downscoped","issued_token_type":"urn:ietf:params:oauth:token-type:access_token","token_type":"Bearer","expires_in":600}"#)
            .create_async()
            .await;
        let source = token("source-token", Utc::now() + TimeDelta::hours(1));
        let before = Utc::now();
        let downscoped = downscope_token(&source, &boundary, &options(&server, "/v1/token"))
            .await
            .unwrap();
        mock.assert_async().await;
        assert_eq!(downscoped.credential.bearer, "downscoped");
        assert!(downscoped.expiration >= before + TimeDelta::seconds(600));
        assert!(downscoped.expiration < source.expiration);
    }

    #[tokio::test]
    async fn downscoped_tokens_never_outlive_their_source() {
        let mut server = mockito::Server::new_async().await;
        let source = token("source-token", Utc::now() + TimeDelta::minutes(5));
        for body in [
            // No expiry reported: the source's.
            r#"{"access_token":"downscoped","token_type":"Bearer"}"#,
            // Longer than the source: clamped to it.
            r#"{"access_token":"downscoped","expires_in":3600}"#,
        ] {
            let mock = server
                .mock("POST", "/v1/token")
                .with_body(body)
                .create_async()
                .await;
            let downscoped = downscope_token(&source, &json!({}), &options(&server, "/v1/token"))
                .await
                .unwrap();
            mock.assert_async().await;
            mock.remove_async().await;
            assert_eq!(downscoped.expiration, source.expiration, "{body}");
        }
    }

    #[tokio::test]
    async fn downscope_errors_do_not_leak_the_source() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/v1/token")
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant"}"#)
            .create_async()
            .await;
        let source = token("source-token-secret", Utc::now() + TimeDelta::hours(1));
        let err = downscope_token(&source, &json!({}), &options(&server, "/v1/token"))
            .await
            .unwrap_err();
        assert!(
            !format!("{err} {err:?}").contains("source-token-secret"),
            "{err}"
        );
        assert!(!format!("{source:?}").contains("source-token-secret"));
    }
}
