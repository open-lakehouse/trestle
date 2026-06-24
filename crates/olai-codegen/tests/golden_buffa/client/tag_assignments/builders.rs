// @generated — do not edit by hand.
#![allow(unused_mut)]
type BoxFut<'a, T> = ::futures::future::BoxFuture<'a, T>;
use std::future::IntoFuture;
use crate::Result;
use example_common::models::tags::v1::*;
use super::client::*;
/// Builder for tag assignments
pub struct ListTagAssignmentsBuilder {
    client: TagAssignmentClient,
    request: ListTagAssignmentsRequest,
}
impl ListTagAssignmentsBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `TagAssignmentClient`.
    pub(crate) fn new(
        client: TagAssignmentClient,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        max_results: i32,
        page_token: impl Into<String>,
    ) -> Self {
        let request = ListTagAssignmentsRequest {
            entity_type: entity_type.into(),
            entity_name: entity_name.into(),
            max_results,
            page_token: page_token.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for ListTagAssignmentsBuilder {
    type Output = Result<ListTagAssignmentsResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.list_tag_assignments(&request).await })
    }
}
/// Builder for tag assignment
pub struct CreateTagAssignmentBuilder {
    client: TagAssignmentClient,
    request: CreateTagAssignmentRequest,
}
impl CreateTagAssignmentBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `TagAssignmentClient`.
    pub(crate) fn new(
        client: TagAssignmentClient,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
    ) -> Self {
        let request = CreateTagAssignmentRequest {
            entity_type: entity_type.into(),
            entity_name: entity_name.into(),
            ..Default::default()
        };
        Self { client, request }
    }
    /// Set tag
    pub fn with_tag(mut self, tag: impl Into<Option<TagAssignment>>) -> Self {
        self.request.tag = tag.into();
        self
    }
}
impl IntoFuture for CreateTagAssignmentBuilder {
    type Output = Result<TagAssignment>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.create_tag_assignment(&request).await })
    }
}
/// Builder for tag assignment
pub struct GetTagAssignmentBuilder {
    client: TagAssignmentClient,
    request: GetTagAssignmentRequest,
}
impl GetTagAssignmentBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `TagAssignmentClient`.
    pub(crate) fn new(
        client: TagAssignmentClient,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> Self {
        let request = GetTagAssignmentRequest {
            entity_type: entity_type.into(),
            entity_name: entity_name.into(),
            tag_key: tag_key.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for GetTagAssignmentBuilder {
    type Output = Result<TagAssignment>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.get_tag_assignment(&request).await })
    }
}
/// Builder for tag assignment
pub struct DeleteTagAssignmentBuilder {
    client: TagAssignmentClient,
    request: DeleteTagAssignmentRequest,
}
impl DeleteTagAssignmentBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `TagAssignmentClient`.
    pub(crate) fn new(
        client: TagAssignmentClient,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> Self {
        let request = DeleteTagAssignmentRequest {
            entity_type: entity_type.into(),
            entity_name: entity_name.into(),
            tag_key: tag_key.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for DeleteTagAssignmentBuilder {
    type Output = Result<DeleteTagAssignmentResponse>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.delete_tag_assignment(&request).await })
    }
}
/// Builder for tag assignment
pub struct TouchTagAssignmentBuilder {
    client: TagAssignmentClient,
    request: TouchTagAssignmentRequest,
}
impl TouchTagAssignmentBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `TagAssignmentClient`.
    pub(crate) fn new(
        client: TagAssignmentClient,
        entity_type: impl Into<String>,
        entity_name: impl Into<String>,
        tag_key: impl Into<String>,
    ) -> Self {
        let request = TouchTagAssignmentRequest {
            entity_type: entity_type.into(),
            entity_name: entity_name.into(),
            tag_key: tag_key.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for TouchTagAssignmentBuilder {
    type Output = Result<()>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.touch_tag_assignment(&request).await })
    }
}
