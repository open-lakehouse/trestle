//! The compute a request runs on: actor identities and trusted compute.
//!
//! A request has one subject, the [`Principal`](crate::Principal) its bearer token
//! authenticates: who the data is accessed for. It may also carry an **actor**, the
//! workload (an engine, a job, a function) that makes the request on the subject's
//! behalf, as in RFC 8693's `act` claim. The actor proves who it is with a credential of
//! its own, never with anything in the subject's token:
//!
//! - an [`ActorVerifier`] turns evidence in the request into an [`ActorIdentity`]. The
//!   [`JwtActorVerifier`] reads a workload token from the
//!   [`ACTOR_TOKEN_HEADER`] (a Kubernetes service account token, a SPIFFE JWT-SVID, a CI
//!   OIDC token). Other verifiers, such as one for X.509-SVIDs over mTLS, read the
//!   connection's evidence from the request extensions instead;
//! - [`TrustedCompute`] decides, from the operator's rules, whether the identity is
//!   trusted compute, and which. The result is the [`Actor`] on the principal.
//!
//! Identities are URIs whatever the method, so the rules don't depend on how the actor
//! proved itself: a SPIFFE ID (`spiffe://prod.example/ns/etl/sa/spark`) as is, any other
//! token subject as `<iss>#<sub>`.
//!
//! An actor never adds privileges: authorization still asks what the subject may do. It
//! lets policies tell trusted compute, which enforces governance on the subject's behalf,
//! from any other client (the `actor` key of [`AuthContext`](crate::AuthContext)).

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{Extensions, HeaderMap, HeaderName};

use crate::authenticator::ConfigError;
use crate::error::{AuthError, InvalidToken};
use crate::jwt::{JwtVerifier, peek_issuer};

/// The header [`JwtActorVerifier`] reads the workload token from, unless configured
/// otherwise.
pub const ACTOR_TOKEN_HEADER: &str = "x-olai-actor-token";

/// The scheme of SPIFFE IDs, which are used as actor ids as they are.
const SPIFFE_SCHEME: &str = "spiffe://";

/// The longest lifetime (`exp - iat`) of an accepted actor token unless configured
/// otherwise: one hour, the default of Kubernetes projected service account tokens.
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(3600);

/// How an actor proved its identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActorMethod {
    /// A workload JWT, verified by a [`JwtActorVerifier`].
    Jwt,
    /// A client certificate presented over mutual TLS, such as an X.509-SVID.
    Mtls,
}

impl ActorMethod {
    /// The method's name in an authorization context: `jwt` or `mtls`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jwt => "jwt",
            Self::Mtls => "mtls",
        }
    }
}

/// A verified actor identity, before [`TrustedCompute`] decides whether it is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActorIdentity {
    /// The actor's id: a URI (see the [module docs](self)).
    pub id: String,
    /// Who vouched for the actor: a token's `iss`, or a certificate's trust domain.
    pub issuer: String,
    /// How the actor proved its identity.
    pub method: ActorMethod,
}

impl ActorIdentity {
    /// An identity `id`, vouched for by `issuer`.
    pub fn new(id: impl Into<String>, issuer: impl Into<String>, method: ActorMethod) -> Self {
        Self {
            id: id.into(),
            issuer: issuer.into(),
            method,
        }
    }
}

/// The compute a request is made from, on a [`Principal`](crate::Principal).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Actor {
    /// The actor's id: a URI (see the [module docs](self)).
    pub id: String,
    /// Who vouched for the actor.
    pub issuer: String,
    /// How the actor proved its identity.
    pub method: ActorMethod,
    /// The trusted compute the actor is, by the id of the [`ComputeRule`] it matched.
    /// `None` for an actor that is not trusted compute.
    pub compute: Option<String>,
}

impl Actor {
    /// Whether the actor is trusted compute.
    pub fn is_trusted(&self) -> bool {
        self.compute.is_some()
    }
}

/// Finds and verifies the actor credential of a request.
///
/// Implementations look at the request's headers or extensions (where a TLS server puts
/// the peer's certificate) and return:
///
/// - `Ok(None)` when the request carries none of their evidence;
/// - `Ok(Some(identity))` when it carries valid evidence;
/// - an error when it carries evidence that is not valid. The request is then rejected,
///   rather than treated as having no actor, so a misconfigured workload is noticed.
#[async_trait]
pub trait ActorVerifier: Send + Sync + 'static {
    /// The actor identity the request proves, if any.
    ///
    /// # Errors
    ///
    /// [`AuthError::InvalidActor`] for invalid evidence, [`AuthError::Unavailable`] when
    /// it can't be checked.
    async fn verify(
        &self,
        headers: &HeaderMap,
        extensions: &Extensions,
    ) -> Result<Option<ActorIdentity>, AuthError>;
}

/// Verifies workload JWTs sent in a header ([`ACTOR_TOKEN_HEADER`] by default).
///
/// The token goes to the [`JwtVerifier`] of its `iss`, which checks the signature, the
/// audience (the service itself, so a token minted for another service is refused) and
/// the expiry. Actor tokens must also be short-lived: they need an `iat`, and `exp - iat`
/// may not exceed the maximum lifetime (one hour unless set with
/// [`with_max_lifetime`](Self::with_max_lifetime)).
///
/// The identity is the token's `sub` when it is a SPIFFE ID (a JWT-SVID), and
/// `<iss>#<sub>` otherwise.
pub struct JwtActorVerifier {
    header: HeaderName,
    issuers: HashMap<String, JwtVerifier>,
    max_lifetime: Duration,
}

impl JwtActorVerifier {
    /// Accepts the tokens of each verifier's issuer.
    ///
    /// # Errors
    ///
    /// [`ConfigError::DuplicateIssuer`] when two verifiers share an issuer.
    pub fn new(verifiers: impl IntoIterator<Item = JwtVerifier>) -> Result<Self, ConfigError> {
        let mut issuers = HashMap::new();
        for verifier in verifiers {
            let issuer = verifier.issuer().to_owned();
            if issuers.insert(issuer.clone(), verifier).is_some() {
                return Err(ConfigError::DuplicateIssuer(issuer));
            }
        }
        Ok(Self {
            header: HeaderName::from_static(ACTOR_TOKEN_HEADER),
            issuers,
            max_lifetime: DEFAULT_MAX_LIFETIME,
        })
    }

    /// Reads the token from `header` instead of [`ACTOR_TOKEN_HEADER`].
    pub fn with_header(mut self, header: HeaderName) -> Self {
        self.header = header;
        self
    }

    /// The longest accepted lifetime (`exp - iat`) of a token.
    pub fn with_max_lifetime(mut self, max_lifetime: Duration) -> Self {
        self.max_lifetime = max_lifetime;
        self
    }

    async fn verify_token(&self, token: &str) -> Result<ActorIdentity, AuthError> {
        let actor = |reason: InvalidToken| AuthError::InvalidActor(reason);
        if token.bytes().filter(|&b| b == b'.').count() != 2 {
            return Err(actor(InvalidToken::Malformed));
        }
        let issuer = peek_issuer(token).map_err(actor)?;
        let verifier = self
            .issuers
            .get(&issuer)
            .ok_or(actor(InvalidToken::Issuer))?;
        let claims = verifier.verify(token).await.map_err(|error| match error {
            AuthError::InvalidToken(reason) => actor(reason),
            other => other,
        })?;
        let lifetime = claims
            .issued_at
            .zip(claims.expires_at)
            .map(|(iat, exp)| exp.saturating_sub(iat));
        match lifetime {
            Some(lifetime) if lifetime <= self.max_lifetime.as_secs() as i64 => {}
            _ => return Err(actor(InvalidToken::Lifetime)),
        }
        let id = if claims.sub.starts_with(SPIFFE_SCHEME) {
            claims.sub
        } else {
            format!("{}#{}", claims.iss, claims.sub)
        };
        Ok(ActorIdentity::new(id, claims.iss, ActorMethod::Jwt))
    }
}

#[async_trait]
impl ActorVerifier for JwtActorVerifier {
    async fn verify(
        &self,
        headers: &HeaderMap,
        _extensions: &Extensions,
    ) -> Result<Option<ActorIdentity>, AuthError> {
        let mut values = headers.get_all(&self.header).iter();
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(AuthError::InvalidActor(InvalidToken::Malformed));
        }
        let token = value
            .to_str()
            .map_err(|_| AuthError::InvalidActor(InvalidToken::Malformed))?
            .trim();
        self.verify_token(token).await.map(Some)
    }
}

/// How a [`ComputeRule`] matches actor ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorMatch {
    /// The id is exactly this.
    Exact(String),
    /// The id is this, or is under it: the prefix is followed by a `/`, or itself ends
    /// in `/`, `#` or `:`. So `spiffe://prod.example/ns/etl` matches
    /// `spiffe://prod.example/ns/etl/sa/spark` but not `spiffe://prod.example/ns/etl-test`.
    Prefix(String),
}

impl ActorMatch {
    fn matches(&self, id: &str) -> bool {
        match self {
            Self::Exact(exact) => id == exact,
            Self::Prefix(prefix) => match id.strip_prefix(prefix.as_str()) {
                None => false,
                Some("") => true,
                Some(rest) => prefix.ends_with(['/', '#', ':']) || rest.starts_with('/'),
            },
        }
    }
}

/// One kind of trusted compute: the actors it covers, and its id in policies and audit
/// records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeRule {
    compute: String,
    matcher: ActorMatch,
    issuer: Option<String>,
}

impl ComputeRule {
    /// The trusted compute `compute`: the actors whose id `matcher` matches.
    pub fn new(compute: impl Into<String>, matcher: ActorMatch) -> Self {
        Self {
            compute: compute.into(),
            matcher,
            issuer: None,
        }
    }

    /// Only covers actors vouched for by `issuer`.
    ///
    /// Set it for SPIFFE IDs when several issuers are accepted: a SPIFFE ID is used as
    /// is, so without it any accepted issuer could vouch for it.
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    /// The compute's id.
    pub fn compute(&self) -> &str {
        &self.compute
    }

    fn matches(&self, identity: &ActorIdentity) -> bool {
        self.issuer
            .as_ref()
            .is_none_or(|issuer| *issuer == identity.issuer)
            && self.matcher.matches(&identity.id)
    }
}

/// The operator's rules for which actors are trusted compute. The first matching rule
/// wins. With no rules, no actor is trusted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedCompute {
    rules: Vec<ComputeRule>,
}

impl TrustedCompute {
    /// No trusted compute.
    pub fn new() -> Self {
        Self::default()
    }

    /// Also trusts the actors `rule` covers.
    pub fn with(mut self, rule: ComputeRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// The rules, in order.
    pub fn rules(&self) -> &[ComputeRule] {
        &self.rules
    }

    /// The actor `identity` is: trusted compute when a rule covers it.
    pub fn resolve(&self, identity: ActorIdentity) -> Actor {
        let compute = self
            .rules
            .iter()
            .find(|rule| rule.matches(&identity))
            .map(|rule| rule.compute.clone());
        Actor {
            id: identity.id,
            issuer: identity.issuer,
            method: identity.method,
            compute,
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use serde_json::{Value, json};

    use super::*;
    use crate::key_source::PushKeySource;
    use crate::test_support::{AUDIENCE, NOW, jwks, sign, signing_key};

    const WORKLOAD_ISSUER: &str = "https://oidc.eks.example.com/id/ABC";

    fn verifier() -> JwtActorVerifier {
        let keys = PushKeySource::new();
        keys.set_jwks(&jwks(&[("w1", 9)])).unwrap();
        let workload = JwtVerifier::new(WORKLOAD_ISSUER, keys)
            .with_audience(AUDIENCE)
            .with_clock(|| NOW);
        JwtActorVerifier::new([workload])
            .unwrap()
            .with_max_lifetime(Duration::from_secs(600))
    }

    fn workload_claims(sub: &str) -> Value {
        json!({
            "iss": WORKLOAD_ISSUER,
            "aud": AUDIENCE,
            "sub": sub,
            "iat": NOW - 60,
            "exp": NOW + 240,
        })
    }

    fn workload_token(claims: &Value) -> String {
        sign(
            &json!({ "alg": "EdDSA", "kid": "w1" }),
            claims,
            &signing_key(9),
        )
    }

    fn headers(tokens: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for token in tokens {
            headers.append(ACTOR_TOKEN_HEADER, HeaderValue::from_str(token).unwrap());
        }
        headers
    }

    async fn verify(tokens: &[&str]) -> Result<Option<ActorIdentity>, AuthError> {
        verifier()
            .verify(&headers(tokens), &Extensions::new())
            .await
    }

    fn invalid(result: Result<Option<ActorIdentity>, AuthError>) -> Option<InvalidToken> {
        match result {
            Err(AuthError::InvalidActor(reason)) => Some(reason),
            _ => None,
        }
    }

    #[tokio::test]
    async fn no_header_is_no_actor() {
        assert_eq!(verify(&[]).await.unwrap(), None);
    }

    #[tokio::test]
    async fn identifies_workloads_by_uri() {
        let token = workload_token(&workload_claims("system:serviceaccount:etl:spark"));
        assert_eq!(
            verify(&[&token]).await.unwrap(),
            Some(ActorIdentity::new(
                format!("{WORKLOAD_ISSUER}#system:serviceaccount:etl:spark"),
                WORKLOAD_ISSUER,
                ActorMethod::Jwt
            ))
        );
        // A JWT-SVID's subject is a SPIFFE ID, used as is.
        let svid = workload_token(&workload_claims("spiffe://prod.example/ns/etl/sa/spark"));
        let identity = verify(&[&svid]).await.unwrap().unwrap();
        assert_eq!(identity.id, "spiffe://prod.example/ns/etl/sa/spark");
    }

    #[tokio::test]
    async fn rejects_invalid_tokens() {
        let mut expired = workload_claims("spark");
        expired["exp"] = json!(NOW - 3600);
        expired["iat"] = json!(NOW - 3700);
        let mut other_audience = workload_claims("spark");
        other_audience["aud"] = json!("https://other-service.example.com");
        let mut unknown_issuer = workload_claims("spark");
        unknown_issuer["iss"] = json!("https://evil.example.com");
        let forged = sign(
            &json!({ "alg": "EdDSA", "kid": "w1" }),
            &workload_claims("spark"),
            &signing_key(1),
        );
        let cases = [
            (workload_token(&expired), InvalidToken::Expired),
            (workload_token(&other_audience), InvalidToken::Audience),
            (workload_token(&unknown_issuer), InvalidToken::Issuer),
            (forged, InvalidToken::Signature),
            ("olk_not_a_jwt".to_owned(), InvalidToken::Malformed),
        ];
        for (token, reason) in cases {
            assert_eq!(invalid(verify(&[&token]).await), Some(reason), "{reason}");
        }
    }

    #[tokio::test]
    async fn requires_short_lived_tokens() {
        let mut long = workload_claims("spark");
        long["exp"] = json!(NOW + 3600);
        let mut no_iat = workload_claims("spark");
        no_iat.as_object_mut().unwrap().remove("iat");
        for claims in [long, no_iat] {
            assert_eq!(
                invalid(verify(&[&workload_token(&claims)]).await),
                Some(InvalidToken::Lifetime)
            );
        }
    }

    #[tokio::test]
    async fn rejects_more_than_one_token() {
        let token = workload_token(&workload_claims("spark"));
        assert_eq!(
            invalid(verify(&[&token, &token]).await),
            Some(InvalidToken::Malformed)
        );
    }

    #[test]
    fn rejects_duplicate_issuers() {
        let workload = || JwtVerifier::new(WORKLOAD_ISSUER, PushKeySource::new());
        assert!(matches!(
            JwtActorVerifier::new([workload(), workload()]),
            Err(ConfigError::DuplicateIssuer(_))
        ));
    }

    #[test]
    fn prefixes_match_on_segment_boundaries() {
        let prefix = ActorMatch::Prefix("spiffe://prod.example/ns/etl".into());
        assert!(prefix.matches("spiffe://prod.example/ns/etl"));
        assert!(prefix.matches("spiffe://prod.example/ns/etl/sa/spark"));
        assert!(!prefix.matches("spiffe://prod.example/ns/etl-test/sa/spark"));
        assert!(!prefix.matches("spiffe://prod.example/ns"));
        let fragment = ActorMatch::Prefix("https://oidc.example.com#fn:".into());
        assert!(fragment.matches("https://oidc.example.com#fn:ingest"));
        assert!(!fragment.matches("https://oidc.example.com#user:alice"));
        let exact = ActorMatch::Exact("spiffe://prod.example/ns/etl".into());
        assert!(!exact.matches("spiffe://prod.example/ns/etl/sa/spark"));
    }

    #[test]
    fn resolves_trusted_compute() {
        let trusted = TrustedCompute::new()
            .with(ComputeRule::new(
                "spark-prod",
                ActorMatch::Prefix("spiffe://prod.example/ns/etl".into()),
            ))
            .with(
                ComputeRule::new(
                    "pinned",
                    ActorMatch::Exact("spiffe://prod.example/ns/jobs/sa/nightly".into()),
                )
                .with_issuer(WORKLOAD_ISSUER),
            );

        let spark = trusted.resolve(ActorIdentity::new(
            "spiffe://prod.example/ns/etl/sa/spark",
            WORKLOAD_ISSUER,
            ActorMethod::Mtls,
        ));
        assert_eq!(spark.compute.as_deref(), Some("spark-prod"));
        assert!(spark.is_trusted());
        assert_eq!(spark.method, ActorMethod::Mtls);

        let pinned = |issuer| {
            trusted.resolve(ActorIdentity::new(
                "spiffe://prod.example/ns/jobs/sa/nightly",
                issuer,
                ActorMethod::Jwt,
            ))
        };
        assert!(pinned(WORKLOAD_ISSUER).is_trusted());
        assert!(!pinned("https://other.example.com").is_trusted());

        let laptop = trusted.resolve(ActorIdentity::new(
            "https://oidc.example.com#laptop",
            "https://oidc.example.com",
            ActorMethod::Jwt,
        ));
        assert_eq!(laptop.compute, None);
        assert!(!TrustedCompute::new().resolve(spark_identity()).is_trusted());
    }

    fn spark_identity() -> ActorIdentity {
        ActorIdentity::new(
            "spiffe://prod.example/ns/etl/sa/spark",
            WORKLOAD_ISSUER,
            ActorMethod::Jwt,
        )
    }
}
