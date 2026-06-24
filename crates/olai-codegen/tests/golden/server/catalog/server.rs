// @generated — do not edit by hand.
#![allow(unused_mut, clippy::too_many_arguments)]
use crate::Result;
use example_common::models::catalog::v1::*;
use super::handler::CatalogHandler;
use axum::extract::State;
pub async fn create_catalog<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: CreateCatalogRequest,
) -> Result<::axum::Json<Catalog>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.create_catalog(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn get_catalog<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: GetCatalogRequest,
) -> Result<::axum::Json<Catalog>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.get_catalog(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn list_catalogs<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: ListCatalogsRequest,
) -> Result<::axum::Json<ListCatalogsResponse>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.list_catalogs(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn update_catalog<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: UpdateCatalogRequest,
) -> Result<::axum::Json<Catalog>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.update_catalog(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn delete_catalog<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: DeleteCatalogRequest,
) -> Result<::axum::Json<DeleteCatalogResponse>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.delete_catalog(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn generate_catalog_token<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: GenerateCatalogTokenRequest,
) -> Result<::axum::Json<CatalogToken>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.generate_catalog_token(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn get_catalog_status<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: GetCatalogStatusRequest,
) -> Result<::axum::Json<CatalogStatus>>
where
    T: CatalogHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.get_catalog_status(request, context).await?;
    Ok(axum::Json(result))
}
