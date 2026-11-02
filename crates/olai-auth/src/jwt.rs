use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::Signature;
use serde::de::{self, Deserializer};
use serde::{Deserialize, de::DeserializeOwned};

use crate::error::{AuthError, InvalidToken};
use crate::key_source::{KeyCache, KeySource};

/// Tokens longer than this are rejected before any decoding.
const MAX_TOKEN_LEN: usize = 8 * 1024;
/// The default allowance for clock skew on `exp` and `nbf`.
const DEFAULT_LEEWAY: Duration = Duration::from_secs(30);

/// Verifies the JWTs of one issuer.
///
/// A token is accepted when all of these hold:
///
/// - the header's `alg` is `EdDSA` (no other algorithm is accepted) and it has a `kid`;
/// - the Ed25519 signature verifies against that `kid`'s key from the [`KeySource`];
/// - `iss` equals the configured issuer exactly;
/// - `aud` (a string or an array) contains one of the configured audiences;
/// - `exp` is present and not past, and `nbf`, if present, is not in the future, both
///   with a leeway for clock skew (30 seconds unless set with
///   [`with_leeway`](Self::with_leeway)).
///
/// With no audience configured, every token is rejected.
pub struct JwtVerifier {
    issuer: String,
    audiences: Vec<String>,
    leeway: Duration,
    keys: KeyCache,
    clock: fn() -> i64,
}

/// The claims of a verified token that [`Principal`](crate::Principal)s are built from.
///
/// The `role` claim is deliberately not read: identity providers commonly use it for a
/// database or API role rather than an application role, so admin status comes from the
/// [`UserStatusStore`](crate::UserStatusStore) instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JwtClaims {
    /// `iss`.
    pub iss: String,
    /// `sub`.
    pub sub: String,
    /// `email`.
    pub email: Option<String>,
    /// `email_verified`, or `emailVerified` when there is none.
    pub email_verified: bool,
    /// Scopes from `scope` (space-separated), or `scp` (a string or an array) when there
    /// is none.
    pub scopes: Vec<String>,
    /// `client_id`, or `azp` when there is none.
    pub client_id: Option<String>,
}

#[derive(Deserialize)]
struct RawClaims {
    iss: String,
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    #[serde(rename = "emailVerified")]
    email_verified_camel: Option<bool>,
    #[serde(default, deserialize_with = "scopes")]
    scope: Option<Vec<String>>,
    #[serde(default, deserialize_with = "scopes")]
    scp: Option<Vec<String>>,
    client_id: Option<String>,
    azp: Option<String>,
}

impl From<RawClaims> for JwtClaims {
    fn from(raw: RawClaims) -> Self {
        Self {
            iss: raw.iss,
            sub: raw.sub,
            email: raw.email,
            email_verified: raw
                .email_verified
                .or(raw.email_verified_camel)
                .unwrap_or(false),
            scopes: raw.scope.or(raw.scp).unwrap_or_default(),
            client_id: raw.client_id.or(raw.azp),
        }
    }
}

/// The claims checked before [`JwtClaims`] are trusted.
#[derive(Deserialize)]
struct Registered {
    iss: String,
    #[serde(default, deserialize_with = "audiences")]
    aud: Vec<String>,
    #[serde(default, deserialize_with = "numeric_date")]
    exp: Option<i64>,
    #[serde(default, deserialize_with = "numeric_date")]
    nbf: Option<i64>,
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Deserialize)]
struct IssuerOnly {
    iss: String,
}

fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl JwtVerifier {
    /// A verifier for tokens issued by `issuer`, signed with keys from `keys`.
    ///
    /// Add the accepted audiences with [`with_audience`](Self::with_audience).
    pub fn new(issuer: impl Into<String>, keys: impl KeySource) -> Self {
        Self {
            issuer: issuer.into(),
            audiences: Vec::new(),
            leeway: DEFAULT_LEEWAY,
            keys: KeyCache::new(keys),
            clock: system_now,
        }
    }

    /// Also accepts tokens whose `aud` contains `audience`.
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audiences.push(audience.into());
        self
    }

    /// The allowance for clock skew when checking `exp` and `nbf`.
    pub fn with_leeway(mut self, leeway: Duration) -> Self {
        self.leeway = leeway;
        self
    }

    /// The least time between two key refreshes (30 seconds by default).
    ///
    /// A token with an unknown `kid` refreshes the keys, unless the last refresh was more
    /// recent than this. That bounds the fetches made on behalf of forged tokens.
    pub fn with_min_refresh_interval(mut self, interval: Duration) -> Self {
        self.keys.min_refresh_interval = interval;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: fn() -> i64) -> Self {
        self.clock = clock;
        self
    }

    /// The issuer this verifier accepts.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Verifies `token` and returns its claims.
    ///
    /// # Errors
    ///
    /// [`AuthError::InvalidToken`] naming the failed check, or [`AuthError::Unavailable`]
    /// when the keys could not be fetched.
    pub async fn verify(&self, token: &str) -> Result<JwtClaims, AuthError> {
        let parts = split(token)?;
        let header: Header = decode_json(parts.header)?;
        if header.alg != "EdDSA" {
            return Err(InvalidToken::Algorithm.into());
        }
        let kid = header.kid.ok_or(InvalidToken::MissingKeyId)?;
        let signature = URL_SAFE_NO_PAD
            .decode(parts.signature)
            .ok()
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(InvalidToken::Malformed)?;
        let key = self.keys.key(&kid).await?;
        key.verify_strict(parts.signing_input.as_bytes(), &signature)
            .map_err(|_| InvalidToken::Signature)?;

        // Signed by the issuer: the claims can be read now.
        let payload = decode(parts.payload)?;
        let registered: Registered = parse(&payload)?;
        if registered.iss != self.issuer {
            return Err(InvalidToken::Issuer.into());
        }
        if !registered
            .aud
            .iter()
            .any(|aud| self.audiences.iter().any(|allowed| allowed == aud))
        {
            return Err(InvalidToken::Audience.into());
        }
        let now = (self.clock)();
        let leeway = self.leeway.as_secs() as i64;
        match registered.exp {
            Some(exp) if now <= exp.saturating_add(leeway) => {}
            _ => return Err(InvalidToken::Expired.into()),
        }
        if registered
            .nbf
            .is_some_and(|nbf| now < nbf.saturating_sub(leeway))
        {
            return Err(InvalidToken::NotYetValid.into());
        }
        Ok(parse::<RawClaims>(&payload)?.into())
    }
}

/// The token's `iss`, read without verifying anything, to pick the verifier.
pub(crate) fn peek_issuer(token: &str) -> Result<String, InvalidToken> {
    let parts = split(token)?;
    Ok(decode_json::<IssuerOnly>(parts.payload)?.iss)
}

struct Parts<'a> {
    header: &'a str,
    payload: &'a str,
    signature: &'a str,
    signing_input: &'a str,
}

fn split(token: &str) -> Result<Parts<'_>, InvalidToken> {
    if token.len() > MAX_TOKEN_LEN {
        return Err(InvalidToken::Malformed);
    }
    let (signing_input, signature) = token.rsplit_once('.').ok_or(InvalidToken::Malformed)?;
    let (header, payload) = signing_input
        .split_once('.')
        .ok_or(InvalidToken::Malformed)?;
    if payload.contains('.') {
        return Err(InvalidToken::Malformed);
    }
    Ok(Parts {
        header,
        payload,
        signature,
        signing_input,
    })
}

fn decode(segment: &str) -> Result<Vec<u8>, InvalidToken> {
    URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| InvalidToken::Malformed)
}

fn parse<T: DeserializeOwned>(json: &[u8]) -> Result<T, InvalidToken> {
    serde_json::from_slice(json).map_err(|_| InvalidToken::Malformed)
}

fn decode_json<T: DeserializeOwned>(segment: &str) -> Result<T, InvalidToken> {
    parse(&decode(segment)?)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

fn audiences<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(aud) => vec![aud],
        OneOrMany::Many(auds) => auds,
    })
}

fn scopes<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    Ok(
        Option::<OneOrMany>::deserialize(d)?.map(|scopes| match scopes {
            OneOrMany::One(scope) => scope.split_whitespace().map(str::to_owned).collect(),
            OneOrMany::Many(scopes) => scopes,
        }),
    )
}

/// A NumericDate (RFC 7519): seconds since the epoch, possibly fractional.
fn numeric_date<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let Some(n) = Option::<serde_json::Number>::deserialize(d)? else {
        return Ok(None);
    };
    n.as_i64()
        .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
        .map(Some)
        .ok_or_else(|| de::Error::custom("invalid NumericDate"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::key_source::PushKeySource;
    use crate::test_support::{AUDIENCE, ISSUER, NOW, claims, jwks, sign, signing_key, token};

    fn verifier() -> JwtVerifier {
        let keys = PushKeySource::new();
        keys.set_jwks(&jwks(&[("k1", 1), ("k2", 2)])).unwrap();
        JwtVerifier::new(ISSUER, keys)
            .with_audience(ISSUER)
            .with_audience(AUDIENCE)
            .with_min_refresh_interval(Duration::ZERO)
            .with_clock(|| NOW)
    }

    async fn reason(token: &str) -> InvalidToken {
        match verifier().verify(token).await {
            Err(AuthError::InvalidToken(reason)) => reason,
            other => panic!("expected an invalid token, got {other:?}"),
        }
    }

    fn with(changes: serde_json::Value) -> serde_json::Value {
        let mut claims = claims();
        for (k, v) in changes.as_object().unwrap() {
            if v.is_null() {
                claims.as_object_mut().unwrap().remove(k);
            } else {
                claims[k] = v.clone();
            }
        }
        claims
    }

    #[tokio::test]
    async fn accepts_a_valid_token() {
        let claims = verifier().verify(&token(&claims())).await.unwrap();
        assert_eq!(claims.iss, ISSUER);
        assert_eq!(claims.sub, "8c1d0f5e-3b7a-4c55-9f40-2d6f1e9b7a10");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert!(claims.email_verified);
        assert!(claims.scopes.is_empty());
    }

    #[tokio::test]
    async fn reads_scopes_and_client_id() {
        let t = token(&with(json!({ "scope": "read write", "client_id": "cli" })));
        let claims = verifier().verify(&t).await.unwrap();
        assert_eq!(claims.scopes, ["read", "write"]);
        assert_eq!(claims.client_id.as_deref(), Some("cli"));

        let t = token(&with(json!({ "scp": ["read"], "azp": "spa" })));
        let claims = verifier().verify(&t).await.unwrap();
        assert_eq!(claims.scopes, ["read"]);
        assert_eq!(claims.client_id.as_deref(), Some("spa"));

        // Both spellings at once: the standard claim wins.
        let t = token(&with(json!({
            "scope": "read", "scp": "write", "client_id": "cli", "azp": "spa",
            "email_verified": false,
        })));
        let claims = verifier().verify(&t).await.unwrap();
        assert_eq!(claims.scopes, ["read"]);
        assert_eq!(claims.client_id.as_deref(), Some("cli"));
        assert!(!claims.email_verified);
    }

    #[tokio::test]
    async fn rejects_the_wrong_issuer() {
        let t = token(&with(json!({ "iss": "https://auth.example.com/" })));
        assert_eq!(reason(&t).await, InvalidToken::Issuer);
    }

    #[tokio::test]
    async fn checks_the_audience_allow_list() {
        let t = token(&with(json!({ "aud": "https://other.example.com" })));
        assert_eq!(reason(&t).await, InvalidToken::Audience);
        let t = token(&with(json!({ "aud": null })));
        assert_eq!(reason(&t).await, InvalidToken::Audience);
        let t = token(&with(
            json!({ "aud": ["https://other.example.com", AUDIENCE] }),
        ));
        assert!(verifier().verify(&t).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_everything_without_an_audience() {
        let keys = PushKeySource::new();
        keys.set_jwks(&jwks(&[("k1", 1)])).unwrap();
        let verifier = JwtVerifier::new(ISSUER, keys).with_clock(|| NOW);
        assert!(matches!(
            verifier.verify(&token(&claims())).await,
            Err(AuthError::InvalidToken(InvalidToken::Audience))
        ));
    }

    #[tokio::test]
    async fn pins_the_algorithm() {
        for alg in ["none", "HS256", "RS256", "ES256", "eddsa"] {
            let t = sign(
                &json!({ "alg": alg, "kid": "k1" }),
                &claims(),
                &signing_key(1),
            );
            assert_eq!(reason(&t).await, InvalidToken::Algorithm, "{alg}");
        }
        // An unsigned token with an empty signature.
        let t = token(&claims());
        let (input, _) = t.rsplit_once('.').unwrap();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let (_, payload) = input.split_once('.').unwrap();
        assert_eq!(
            reason(&format!("{header}.{payload}.")).await,
            InvalidToken::Algorithm
        );
    }

    #[tokio::test]
    async fn requires_a_known_key_id() {
        let t = sign(&json!({ "alg": "EdDSA" }), &claims(), &signing_key(1));
        assert_eq!(reason(&t).await, InvalidToken::MissingKeyId);
        let t = sign(
            &json!({ "alg": "EdDSA", "kid": "k9" }),
            &claims(),
            &signing_key(1),
        );
        assert_eq!(reason(&t).await, InvalidToken::UnknownKey);
    }

    #[tokio::test]
    async fn rejects_a_key_that_does_not_match_the_kid() {
        let t = sign(
            &json!({ "alg": "EdDSA", "kid": "k2" }),
            &claims(),
            &signing_key(1),
        );
        assert_eq!(reason(&t).await, InvalidToken::Signature);
    }

    #[tokio::test]
    async fn checks_expiry_with_leeway() {
        let t = token(&with(json!({ "exp": NOW - 31 })));
        assert_eq!(reason(&t).await, InvalidToken::Expired);
        let t = token(&with(json!({ "exp": NOW - 30 })));
        assert!(verifier().verify(&t).await.is_ok());
        let t = token(&with(json!({ "exp": null })));
        assert_eq!(reason(&t).await, InvalidToken::Expired);
        let t = token(&with(json!({ "exp": (NOW + 60) as f64 + 0.5 })));
        assert!(verifier().verify(&t).await.is_ok());
    }

    #[tokio::test]
    async fn checks_not_before_with_leeway() {
        let t = token(&with(json!({ "nbf": NOW + 31 })));
        assert_eq!(reason(&t).await, InvalidToken::NotYetValid);
        let t = token(&with(json!({ "nbf": NOW + 30 })));
        assert!(verifier().verify(&t).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_tampering() {
        let t = token(&claims());
        let (header, rest) = t.split_once('.').unwrap();
        let (_, signature) = rest.split_once('.').unwrap();

        // The payload of another, unsigned set of claims under the original signature.
        let forged = URL_SAFE_NO_PAD.encode(with(json!({ "sub": "admin" })).to_string());
        assert_eq!(
            reason(&format!("{header}.{forged}.{signature}")).await,
            InvalidToken::Signature
        );

        let mut bytes = URL_SAFE_NO_PAD.decode(signature).unwrap();
        bytes[0] ^= 1;
        let flipped = URL_SAFE_NO_PAD.encode(bytes);
        let (input, _) = t.rsplit_once('.').unwrap();
        assert_eq!(
            reason(&format!("{input}.{flipped}")).await,
            InvalidToken::Signature
        );
    }

    #[tokio::test]
    async fn rejects_malformed_tokens() {
        let t = token(&claims());
        for bad in [
            "",
            "abc",
            "a.b",
            "a.b.c.d",
            &format!("{t}.x"),
            &format!("{t}="),
            &format!("{}A", t),
            &"a".repeat(MAX_TOKEN_LEN + 1),
        ] {
            assert_eq!(reason(bad).await, InvalidToken::Malformed, "{bad:.40}");
        }
    }

    #[test]
    fn peeks_the_issuer_without_verifying() {
        let t = sign(&json!({ "alg": "none" }), &claims(), &signing_key(9));
        assert_eq!(peek_issuer(&t).unwrap(), ISSUER);
        assert_eq!(peek_issuer("a.b.c"), Err(InvalidToken::Malformed));
    }
}
