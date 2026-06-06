// @generated — do not edit by hand.
#![allow(unused_mut)]
use crate::Result;
use crate::models::catalog::v1::*;
use axum::{RequestExt, RequestPartsExt};
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ListByTagsRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        #[derive(serde::Deserialize)]
        struct QueryParams {
            #[serde(default)]
            tags: Vec<String>,
            max_results: i32,
        }
        let axum_extra::extract::Query(QueryParams { tags, max_results }) = parts
            .extract::<axum_extra::extract::Query<QueryParams>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(ListByTagsRequest {
            tags,
            max_results,
        })
    }
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ListByCatalogTypeRequest {
    type Rejection = axum::response::Response;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        #[derive(serde::Deserialize)]
        struct QueryParams {
            catalog_type: CatalogType,
        }
        let axum_extra::extract::Query(QueryParams { catalog_type }) = parts
            .extract::<axum_extra::extract::Query<QueryParams>>()
            .await
            .map_err(axum::response::IntoResponse::into_response)?;
        Ok(ListByCatalogTypeRequest {
            catalog_type: catalog_type as i32,
        })
    }
}
