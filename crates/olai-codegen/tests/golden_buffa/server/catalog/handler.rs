// @generated — do not edit by hand.
//! Handler trait for [`CatalogHandler`].
//!
//! Implement this trait to provide a custom backend for this service, then mount the
//! generated handler functions (in the sibling `server` module) onto an `axum::Router`
//! with your implementation as state.
//!
//! # Composability
//!
//! A single struct can implement multiple handler traits to serve multiple
//! services. Use [`axum::Router::merge`] to compose per-service routers together.
use async_trait::async_trait;
use crate::Result;
use example_common::models::catalog::v1::*;
#[async_trait]
pub trait CatalogHandler<Cx = crate::Context>: Send + Sync + 'static {
    async fn create_catalog(
        &self,
        request: CreateCatalogRequest,
        context: Cx,
    ) -> Result<Catalog>;
    async fn get_catalog(
        &self,
        request: GetCatalogRequest,
        context: Cx,
    ) -> Result<Catalog>;
    async fn list_catalogs(
        &self,
        request: ListCatalogsRequest,
        context: Cx,
    ) -> Result<ListCatalogsResponse>;
    async fn update_catalog(
        &self,
        request: UpdateCatalogRequest,
        context: Cx,
    ) -> Result<Catalog>;
    async fn delete_catalog(
        &self,
        request: DeleteCatalogRequest,
        context: Cx,
    ) -> Result<DeleteCatalogResponse>;
    /// Custom POST RPC without path params — covers `RequestType::Custom(Post)`
    /// dispatched as a collection method (the shape used by factory-style
    /// RPCs like `GenerateTemporary*Credentials`).
    async fn generate_catalog_token(
        &self,
        request: GenerateCatalogTokenRequest,
        context: Cx,
    ) -> Result<CatalogToken>;
    /// Resource-targeted custom GET (path param, not a collection method) — exercises surfacing a
    /// custom read on the scoped client (`catalog.get_catalog_status()`) instead of leaving its
    /// generated builder orphaned.
    async fn get_catalog_status(
        &self,
        request: GetCatalogStatusRequest,
        context: Cx,
    ) -> Result<CatalogStatus>;
}
