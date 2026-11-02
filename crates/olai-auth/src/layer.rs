use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;

use crate::authenticator::Authenticator;
use crate::error::AuthError;

/// Requires every route of `router` to be called with a valid bearer token.
///
/// An authenticated request gets its [`Principal`](crate::Principal) inserted into the
/// request extensions, where handlers read it with the `Principal` extractor. Any other
/// request is answered by [`Authenticator::reject`] without reaching a handler.
///
/// The layer covers the routes added to `router` before the call. Merge public routes,
/// such as a health check or the
/// [`ProtectedResourceMetadata`](crate::ProtectedResourceMetadata) document, into the
/// app separately so they stay reachable without a token. See the
/// [crate example](crate#example).
pub fn auth_layer<S>(router: Router<S>, authenticator: Authenticator) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(from_fn_with_state(authenticator, authenticate))
}

async fn authenticate(
    State(authenticator): State<Authenticator>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticator
        .authenticate(request.headers().get(AUTHORIZATION))
        .await
    {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(error) => {
            match &error {
                AuthError::Unavailable(source) => {
                    tracing::warn!(error = %source, "authentication backend unavailable");
                }
                _ => tracing::debug!(%error, "request not authenticated"),
            }
            authenticator.reject(&error)
        }
    }
}
