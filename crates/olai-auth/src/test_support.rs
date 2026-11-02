//! Deterministic keys and hand-signed tokens for the unit tests.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{Value, json};

pub(crate) const ISSUER: &str = "https://auth.example.com";
pub(crate) const AUDIENCE: &str = "https://api.example.com";
/// The fixed "now" the unit tests' verifiers run at.
pub(crate) const NOW: i64 = 1_800_000_000;

pub(crate) fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// A JWKS document with one Ed25519 key per `(kid, seed)`.
pub(crate) fn jwks(keys: &[(&str, u8)]) -> Vec<u8> {
    let keys: Vec<Value> = keys
        .iter()
        .map(|(kid, seed)| {
            json!({
                "kty": "OKP",
                "crv": "Ed25519",
                "alg": "EdDSA",
                "kid": kid,
                "x": URL_SAFE_NO_PAD.encode(signing_key(*seed).verifying_key().as_bytes()),
            })
        })
        .collect();
    json!({ "keys": keys }).to_string().into_bytes()
}

/// Encodes and signs `header` and `claims` as a compact JWS.
pub(crate) fn sign(header: &Value, claims: &Value, key: &SigningKey) -> String {
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = key.sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// Claims that pass the test verifier at [`NOW`].
pub(crate) fn claims() -> Value {
    json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "sub": "8c1d0f5e-3b7a-4c55-9f40-2d6f1e9b7a10",
        "email": "user@example.com",
        "emailVerified": true,
        "role": "authenticated",
        "iat": NOW - 60,
        "exp": NOW + 840,
    })
}

/// A valid token signed with seed `1` under `kid` `k1`.
pub(crate) fn token(claims: &Value) -> String {
    sign(
        &json!({ "alg": "EdDSA", "kid": "k1" }),
        claims,
        &signing_key(1),
    )
}
