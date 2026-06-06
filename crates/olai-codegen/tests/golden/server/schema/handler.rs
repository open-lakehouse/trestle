// @generated — do not edit by hand.
//! Handler trait for [`SchemaHandler`].
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
pub trait SchemaHandler<Cx = crate::Context>: Send + Sync + 'static {
    /// Repeated string query param
    async fn list_by_tags(
        &self,
        request: ListByTagsRequest,
        context: Cx,
    ) -> Result<ListByTagsResponse>;
    /// Enum query param
    async fn list_by_catalog_type(
        &self,
        request: ListByCatalogTypeRequest,
        context: Cx,
    ) -> Result<ListByTagsResponse>;
}
