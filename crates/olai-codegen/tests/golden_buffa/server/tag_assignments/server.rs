// @generated — do not edit by hand.
#![allow(unused_mut, clippy::too_many_arguments)]
use crate::Result;
use example_common::models::tags::v1::*;
use super::handler::TagAssignmentHandler;
use axum::extract::State;
pub async fn list_tag_assignments<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: ListTagAssignmentsRequest,
) -> Result<::axum::Json<ListTagAssignmentsResponse>>
where
    T: TagAssignmentHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.list_tag_assignments(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn create_tag_assignment<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: CreateTagAssignmentRequest,
) -> Result<::axum::Json<TagAssignment>>
where
    T: TagAssignmentHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.create_tag_assignment(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn get_tag_assignment<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: GetTagAssignmentRequest,
) -> Result<::axum::Json<TagAssignment>>
where
    T: TagAssignmentHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.get_tag_assignment(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn delete_tag_assignment<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: DeleteTagAssignmentRequest,
) -> Result<::axum::Json<DeleteTagAssignmentResponse>>
where
    T: TagAssignmentHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.delete_tag_assignment(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn touch_tag_assignment<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: TouchTagAssignmentRequest,
) -> Result<()>
where
    T: TagAssignmentHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    handler.touch_tag_assignment(request, context).await?;
    Ok(())
}
