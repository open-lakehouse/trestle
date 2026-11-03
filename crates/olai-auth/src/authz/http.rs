use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::Authorizer;
use super::wire::{
    AUTHORIZE_BATCH_PATH, AUTHORIZE_PATH, AuthorizeRequest, AuthorizeResponse,
    BatchAuthorizeRequest, BatchAuthorizeResponse,
};

/// How long a decision may take unless configured otherwise: well above a cold PDP.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// An [`Authorizer`] that asks a remote policy decision point (PDP).
///
/// It posts to `<base>/v1/authorize` and `<base>/v1/authorize/batch` (see the
/// [`wire`](crate::wire) types) with `Authorization: Bearer <secret>`, the secret that
/// identifies this service to the PDP.
///
/// It fails closed: a timeout (2 seconds by default), a connection error, a non-2xx
/// status or a body that isn't a valid answer is a deny, with the reason in
/// [`errors`](AuthorizeResponse::errors). Wrap it in a
/// [`CachedAuthorizer`](crate::CachedAuthorizer) to save round trips.
#[derive(Clone)]
pub struct HttpAuthorizer {
    client: reqwest::Client,
    single_url: String,
    batch_url: String,
    authorization: String,
    timeout: Duration,
}

impl fmt::Debug for HttpAuthorizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpAuthorizer")
            .field("single_url", &self.single_url)
            .field("batch_url", &self.batch_url)
            .field("authorization", &"<redacted>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl HttpAuthorizer {
    /// Asks the PDP at `base_url` (e.g. `https://authz.example.com`), authenticating
    /// with `secret`.
    pub fn new(base_url: impl AsRef<str>, secret: impl AsRef<str>) -> Self {
        Self::with_client(reqwest::Client::new(), base_url, secret)
    }

    /// Like [`new`](Self::new), with the host's HTTP client.
    pub fn with_client(
        client: reqwest::Client,
        base_url: impl AsRef<str>,
        secret: impl AsRef<str>,
    ) -> Self {
        let base = base_url.as_ref().trim_end_matches('/');
        Self {
            client,
            single_url: format!("{base}{AUTHORIZE_PATH}"),
            batch_url: format!("{base}{AUTHORIZE_BATCH_PATH}"),
            authorization: format!("Bearer {}", secret.as_ref()),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// How long a call may take before it counts as a deny (2 seconds by default).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<R, String> {
        let body = serde_json::to_vec(body).map_err(|e| format!("encoding the request: {e}"))?;
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, &self.authorization)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    format!("policy decision point timed out after {:?}", self.timeout)
                } else {
                    format!("policy decision point unreachable: {e}")
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("policy decision point answered {status}"));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("reading the decision: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid decision: {e}"))
    }
}

#[async_trait]
impl Authorizer for HttpAuthorizer {
    async fn authorize(&self, request: &AuthorizeRequest) -> AuthorizeResponse {
        self.post(&self.single_url, request)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "authorization failed closed");
                AuthorizeResponse::deny(error)
            })
    }

    async fn authorize_batch(&self, requests: &[AuthorizeRequest]) -> Vec<AuthorizeResponse> {
        if requests.is_empty() {
            return Vec::new();
        }
        let batch = BatchAuthorizeRequest {
            requests: requests.to_vec(),
        };
        let answer = self
            .post::<_, BatchAuthorizeResponse>(&self.batch_url, &batch)
            .await
            .and_then(|answer| {
                if answer.responses.len() == requests.len() {
                    Ok(answer.responses)
                } else {
                    Err(format!(
                        "policy decision point answered {} of {} requests",
                        answer.responses.len(),
                        requests.len()
                    ))
                }
            });
        answer.unwrap_or_else(|error| {
            tracing::warn!(%error, "authorization failed closed");
            vec![AuthorizeResponse::deny(error); requests.len()]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_secret() {
        let authorizer = HttpAuthorizer::new("https://authz.example.com/", "s3cret-token");
        let shown = format!("{authorizer:?}");
        assert!(!shown.contains("s3cret-token"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(
            shown.contains("https://authz.example.com/v1/authorize"),
            "{shown}"
        );
    }
}
