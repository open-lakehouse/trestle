use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::{Deserialize, Serialize};

/// Where [`ProtectedResourceMetadata::router`] serves the document (RFC 9728 §3).
///
/// This is the location for a resource identified by an origin. A resource with a path
/// component publishes at this path followed by its own path instead.
pub const WELL_KNOWN_PATH: &str = "/.well-known/oauth-protected-resource";

/// OAuth 2.0 protected resource metadata (RFC 9728).
///
/// Tells a client which authorization server issues tokens for this resource, and is
/// what the `resource_metadata` parameter of a `WWW-Authenticate` challenge points to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The resource identifier: the value clients send as the `resource` parameter, and
    /// that tokens for this resource carry in `aud`.
    pub resource: String,
    /// The issuers of the authorization servers that grant tokens for the resource.
    pub authorization_servers: Vec<String>,
    /// The scopes the resource understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
    /// How the resource accepts bearer tokens.
    pub bearer_methods_supported: Vec<String>,
    /// A human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
}

impl ProtectedResourceMetadata {
    /// Metadata for `resource`, whose tokens come from `authorization_server`, accepting
    /// tokens in the `Authorization` header only.
    pub fn new(resource: impl Into<String>, authorization_server: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            authorization_servers: vec![authorization_server.into()],
            scopes_supported: Vec::new(),
            bearer_methods_supported: vec!["header".to_owned()],
            resource_name: None,
        }
    }

    /// A router serving this document as JSON at [`WELL_KNOWN_PATH`]. Merge it outside
    /// [`auth_layer`](crate::auth_layer): clients read it before they have a token.
    pub fn router<S>(&self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let body = serde_json::to_string(self).expect("metadata serializes to JSON");
        Router::new().route(
            WELL_KNOWN_PATH,
            get(move || {
                let body = body.clone();
                async move { ([(CONTENT_TYPE, "application/json")], body).into_response() }
            }),
        )
    }
}
