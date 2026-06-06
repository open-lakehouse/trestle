// @generated — do not edit by hand.
//! Handler trait for [`TagAssignmentHandler`].
//!
//! Implement this trait to provide a custom backend for this service, then mount the
//! generated handler functions (in the sibling `server` module) onto an `axum::Router`
//! with your implementation as state.
//!
//! # Composability
//!
//! A single struct can implement multiple handler traits to serve multiple
//! services. Use [`axum::Router::merge`] to compose per-service routers together.
//!
//! Tag-assignment-shaped API: a resource-less service (no `google.api.resource`)
//! keyed by a composite path `(entity_type, entity_name, tag_key)`. Exercises the
//! flat binding lowering — every method lives on the root client and must accept
//! all path params directly.
use async_trait::async_trait;
use crate::Result;
use example_common::models::tags::v1::*;
#[async_trait]
pub trait TagAssignmentHandler<Cx = crate::Context>: Send + Sync + 'static {
    /// List assignments for an entity. Path params: entity_type, entity_name.
    async fn list_tag_assignments(
        &self,
        request: ListTagAssignmentsRequest,
        context: Cx,
    ) -> Result<ListTagAssignmentsResponse>;
    /// Create/assign a tag. Path params: entity_type, entity_name; body: tag.
    async fn create_tag_assignment(
        &self,
        request: CreateTagAssignmentRequest,
        context: Cx,
    ) -> Result<TagAssignment>;
    /// Get a single assignment. Composite key: entity_type, entity_name, tag_key.
    /// Carries a gnostic `operation_id` to exercise annotation-driven binding method naming
    /// (the binding method should be named `fetch_tag_assignment`, not `get_tag_assignment`).
    async fn get_tag_assignment(
        &self,
        request: GetTagAssignmentRequest,
        context: Cx,
    ) -> Result<TagAssignment>;
    /// Delete a single assignment. Composite key path params.
    async fn delete_tag_assignment(
        &self,
        request: DeleteTagAssignmentRequest,
        context: Cx,
    ) -> Result<DeleteTagAssignmentResponse>;
    /// Custom POST RPC targeting a composite key that returns `Empty` — exercises
    /// the `<()>` / void-return path for a resource-less, path-param'd method.
    async fn touch_tag_assignment(
        &self,
        request: TouchTagAssignmentRequest,
        context: Cx,
    ) -> Result<()>;
}
