// @generated — do not edit by hand.
#![allow(unused_mut)]
use crate::models::tags::v1::*;
use axum::{RequestExt, RequestPartsExt};
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ListTagAssignmentsRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Path((entity_type, entity_name)) = parts
            .extract::<axum::extract::Path<(String, String)>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        #[derive(serde::Deserialize)]
        struct QueryParams {
            max_results: i32,
            page_token: String,
        }
        let axum_extra::extract::Query(QueryParams { max_results, page_token }) = parts
            .extract::<axum_extra::extract::Query<QueryParams>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(ListTagAssignmentsRequest {
            entity_type,
            entity_name,
            max_results,
            page_token,
        })
    }
}
impl<S: Send + Sync> axum::extract::FromRequest<S> for CreateTagAssignmentRequest {
    type Rejection = axum::response::Response;
    async fn from_request(
        mut req: axum::extract::Request<axum::body::Body>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        let axum::extract::Path((entity_type, entity_name)) = parts
            .extract::<axum::extract::Path<(String, String)>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body_req = axum::extract::Request::from_parts(parts, body);
        let axum::extract::Json::<CreateTagAssignmentRequest>(body) = body_req
            .extract()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let tag = body.tag;
        Ok(CreateTagAssignmentRequest {
            entity_type,
            entity_name,
            tag,
        })
    }
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for GetTagAssignmentRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Path((entity_type, entity_name, tag_key)) = parts
            .extract::<axum::extract::Path<(String, String, String)>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(GetTagAssignmentRequest {
            entity_type,
            entity_name,
            tag_key,
        })
    }
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for DeleteTagAssignmentRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Path((entity_type, entity_name, tag_key)) = parts
            .extract::<axum::extract::Path<(String, String, String)>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(DeleteTagAssignmentRequest {
            entity_type,
            entity_name,
            tag_key,
        })
    }
}
impl<S: Send + Sync> axum::extract::FromRequest<S> for TouchTagAssignmentRequest {
    type Rejection = axum::response::Response;
    async fn from_request(
        mut req: axum::extract::Request<axum::body::Body>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        let axum::extract::Path((entity_type, entity_name, tag_key)) = parts
            .extract::<axum::extract::Path<(String, String, String)>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        let body_req = axum::extract::Request::from_parts(parts, body);
        Ok(TouchTagAssignmentRequest {
            entity_type,
            entity_name,
            tag_key,
        })
    }
}
