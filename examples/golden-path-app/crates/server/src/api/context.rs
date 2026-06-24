//! Per-request context, extracted from Databricks Apps headers (or local-dev
//! emulation when running behind the trestle Envoy stack).
//!
//! Databricks Apps injects these headers on every request:
//! - `X-Forwarded-Access-Token`: OAuth On-Behalf-Of token for the calling user
//! - `X-Forwarded-User`: the user's `userName`
//! - `X-Forwarded-Email`: the user's email
//!
//! In local-dev mode (`LOCAL_DEV=1`), the local stack injects synthetic values
//! through Envoy header transforms, so the same extraction code works in both
//! environments.

use std::env;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;

/// Identity + auth context for the current request.
///
/// `#[allow(dead_code)]`: the starter handler doesn't read these yet, but real
/// handlers use them (OBO token to call downstream Databricks APIs, identity for
/// audit/authorization). Kept so the scaffold compiles under `-D warnings`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RequestContext {
    /// OBO token used to call downstream Databricks APIs (Unity Catalog, MLflow, …).
    pub access_token: Option<String>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

impl RequestContext {
    /// Populate from headers. In production this is called automatically via
    /// the Axum extractor below. In local-dev, missing OBO headers fall back
    /// to the `DATABRICKS_TOKEN` env var so handler code is environment-agnostic.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let access_token = headers
            .get("x-forwarded-access-token")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or_else(|| {
                if env::var("LOCAL_DEV").as_deref() == Ok("1") {
                    env::var("DATABRICKS_TOKEN").ok()
                } else {
                    None
                }
            });

        Self {
            access_token,
            user_name: headers
                .get("x-forwarded-user")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
                .or_else(|| env::var("DATABRICKS_FORWARDED_USER").ok()),
            user_email: headers
                .get("x-forwarded-email")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
                .or_else(|| env::var("DATABRICKS_FORWARDED_EMAIL").ok()),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(RequestContext::from_headers(&parts.headers))
    }
}