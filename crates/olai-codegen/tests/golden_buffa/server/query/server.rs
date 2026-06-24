// @generated — do not edit by hand.
#![allow(unused_mut, clippy::too_many_arguments)]
use crate::Result;
use example_common::models::catalog::v1::*;
use super::handler::QueryHandler;
use axum::extract::State;
pub async fn list_by_tags<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: ListByTagsRequest,
) -> Result<::axum::Json<ListByTagsResponse>>
where
    T: QueryHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.list_by_tags(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn list_by_catalog_type<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: ListByCatalogTypeRequest,
) -> Result<::axum::Json<ListByTagsResponse>>
where
    T: QueryHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.list_by_catalog_type(request, context).await?;
    Ok(axum::Json(result))
}
