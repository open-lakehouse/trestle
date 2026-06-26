// @generated — do not edit by hand.
#![allow(unused_mut, unused_imports)]
use crate::models::golden_path_app::v1::*;
use axum::{RequestExt, RequestPartsExt};
impl<S: Send + Sync> axum::extract::FromRequest<S> for CreateGreetingRequest {
    type Rejection = axum::response::Response;
    async fn from_request(
        req: axum::extract::Request<axum::body::Body>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Json(request) = req
            .extract()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(request)
    }
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for GetGreetingRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(name) = parts
            .extract::<axum::extract::Path<String>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(GetGreetingRequest {
            name,
            ..Default::default()
        })
    }
}
