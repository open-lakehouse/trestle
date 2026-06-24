// @generated — do not edit by hand.
//! Handler trait for [`GreetingHandler`].
//!
//! Implement this trait to provide a custom backend for this service, then mount the
//! generated handler functions (in the sibling `server` module) onto an `axum::Router`
//! with your implementation as state.
//!
//! # Composability
//!
//! A single struct can implement multiple handler traits to serve multiple
//! services. Use [`axum::Router::merge`] to compose per-service routers together.
use crate::api::Result;
use async_trait::async_trait;
use golden_path_app_common::models::golden_path_app::v1::*;
#[async_trait]
pub trait GreetingHandler<Cx = crate::api::RequestContext>: Send + Sync + 'static {
    /// Create a new greeting.
    async fn create_greeting(
        &self,
        request: CreateGreetingRequest,
        context: Cx,
    ) -> Result<Greeting>;
    /// Fetch a greeting by name.
    async fn get_greeting(&self, request: GetGreetingRequest, context: Cx) -> Result<Greeting>;
}
