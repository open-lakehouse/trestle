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
use example_common::models::schemas::v1::*;
#[async_trait]
pub trait SchemaHandler<Cx = crate::Context>: Send + Sync + 'static {
    async fn create_schema(
        &self,
        request: CreateSchemaRequest,
        context: Cx,
    ) -> Result<Schema>;
    async fn get_schema(&self, request: GetSchemaRequest, context: Cx) -> Result<Schema>;
    async fn list_schemas(
        &self,
        request: ListSchemasRequest,
        context: Cx,
    ) -> Result<ListSchemasResponse>;
    async fn update_schema(
        &self,
        request: UpdateSchemaRequest,
        context: Cx,
    ) -> Result<Schema>;
    async fn delete_schema(
        &self,
        request: DeleteSchemaRequest,
        context: Cx,
    ) -> Result<DeleteSchemaResponse>;
}
