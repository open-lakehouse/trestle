//! `auth_layer` end to end: requests through an axum router.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey};
use olai_auth::{
    ApiKeyRecord, ApiKeyStore, Authenticator, BoxError, JwtVerifier, Principal,
    ProtectedResourceMetadata, PushKeySource, UserStatus, UserStatusStore, WELL_KNOWN_PATH,
    api_key_digest, auth_layer,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ISSUER: &str = "https://auth.example.com";
const RESOURCE: &str = "https://api.example.com";
const METADATA_URL: &str = "https://api.example.com/.well-known/oauth-protected-resource";

/// `banned@` users are banned, `admin@` users are admins, `ghost@` users don't exist.
struct Users;

#[olai_auth::async_trait]
impl UserStatusStore for Users {
    async fn status(&self, _issuer: &str, sub: &str) -> Result<Option<UserStatus>, BoxError> {
        Ok(match sub {
            "ghost" => None,
            "banned" => Some(UserStatus::from_role(None, true)),
            "admin" => Some(UserStatus::from_role(Some("admin"), false)),
            _ => Some(UserStatus::default()),
        })
    }
}

const API_KEY: &str =
    "olk_0123456789abcdef_fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

/// Knows one key, `API_KEY`, for the `ci-bot` service account.
struct Keys;

#[olai_auth::async_trait]
impl ApiKeyStore for Keys {
    async fn lookup(&self, id: &str) -> Result<Option<ApiKeyRecord>, BoxError> {
        Ok((id == "0123456789abcdef").then(|| ApiKeyRecord {
            sha256: api_key_digest(API_KEY),
            account_id: "ci-bot-id".into(),
            account_name: "ci-bot".into(),
            scopes: vec![],
        }))
    }
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[3; 32])
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn token(sub: &str, exp: u64) -> String {
    let header = URL_SAFE_NO_PAD.encode(json!({ "alg": "EdDSA", "kid": "k" }).to_string());
    let claims = json!({ "iss": ISSUER, "aud": RESOURCE, "sub": sub, "exp": exp });
    let input = format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = URL_SAFE_NO_PAD.encode(key().sign(input.as_bytes()).to_bytes());
    format!("{input}.{signature}")
}

async fn whoami(principal: Principal) -> String {
    format!("{} admin={}", principal.subject(), principal.is_admin())
}

fn app() -> Router {
    let keys = PushKeySource::new();
    let jwks = json!({ "keys": [{
        "kty": "OKP", "crv": "Ed25519", "kid": "k",
        "x": URL_SAFE_NO_PAD.encode(key().verifying_key().as_bytes()),
    }]});
    keys.set_jwks(jwks.to_string().as_bytes()).unwrap();
    let verifier = JwtVerifier::new(ISSUER, keys)
        .with_audience(RESOURCE)
        .with_min_refresh_interval(Duration::ZERO);
    let authenticator = Authenticator::builder(Users)
        .issuer(verifier)
        .api_keys(Keys)
        .resource_metadata_url(METADATA_URL)
        .build()
        .unwrap();

    let protected = auth_layer(Router::new().route("/whoami", get(whoami)), authenticator);
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/unguarded", get(whoami))
        .merge(ProtectedResourceMetadata::new(RESOURCE, ISSUER).router())
        .merge(protected)
}

async fn call(path: &str, authorization: Option<&str>) -> Response {
    let mut request = Request::get(path);
    if let Some(value) = authorization {
        request = request.header(AUTHORIZATION, value);
    }
    app()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn challenge(response: &Response) -> &str {
    response
        .headers()
        .get(WWW_AUTHENTICATE)
        .expect("a WWW-Authenticate challenge")
        .to_str()
        .unwrap()
}

#[tokio::test]
async fn valid_token_reaches_the_handler() {
    let response = call(
        "/whoami",
        Some(&format!("Bearer {}", token("admin", now() + 900))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, "admin admin=true");
}

#[tokio::test]
async fn api_key_reaches_the_handler_as_a_service_account() {
    let response = call("/whoami", Some(&format!("Bearer {API_KEY}"))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, "ci-bot-id admin=false");

    let wrong = API_KEY.replace("fedcba98", "00000000");
    let response = call("/whoami", Some(&format!("Bearer {wrong}"))).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let header = challenge(&response).to_owned();
    assert!(
        header.contains(r#"error_description="invalid API key""#),
        "{header}"
    );
    assert!(!header.contains(&wrong));
}

#[tokio::test]
async fn missing_token_gets_a_challenge_without_an_error_code() {
    for authorization in [None, Some("Basic dXNlcjpwdw==")] {
        let response = call("/whoami", authorization).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            challenge(&response),
            format!(r#"Bearer resource_metadata="{METADATA_URL}""#)
        );
    }
}

#[tokio::test]
async fn invalid_token_gets_an_invalid_token_challenge() {
    let expired = token("someone", now() - 3600);
    let response = call("/whoami", Some(&format!("Bearer {expired}"))).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let header = challenge(&response).to_owned();
    assert_eq!(
        header,
        format!(
            r#"Bearer error="invalid_token", error_description="token expired", resource_metadata="{METADATA_URL}""#
        )
    );
    assert!(!header.contains(&expired));
    assert!(!body(response).await.contains(&expired));

    let response = call(
        "/whoami",
        Some(&format!("Bearer {}", token("ghost", now() + 900))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(challenge(&response).contains(r#"error_description="unknown user""#));
}

#[tokio::test]
async fn banned_user_is_forbidden() {
    let response = call(
        "/whoami",
        Some(&format!("Bearer {}", token("banned", now() + 900))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().get(WWW_AUTHENTICATE).is_none());
}

#[tokio::test]
async fn public_routes_need_no_token() {
    let response = call("/health", None).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = call(WELL_KNOWN_PATH, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    let metadata: Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(
        metadata,
        json!({
            "resource": RESOURCE,
            "authorization_servers": [ISSUER],
            "bearer_methods_supported": ["header"],
        })
    );
}

#[tokio::test]
async fn extractor_outside_the_layer_fails_closed() {
    let response = call(
        "/unguarded",
        Some(&format!("Bearer {}", token("admin", now() + 900))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
