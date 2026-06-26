// @generated — do not edit by hand.
#![allow(unused_mut, clippy::too_many_arguments)]
use super::handler::GreetingHandler;
use crate::api::Result;
use axum::extract::State;
use golden_path_app_common::models::golden_path_app::v1::*;
pub async fn create_greeting<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: CreateGreetingRequest,
) -> Result<::axum::Json<Greeting>>
where
    T: GreetingHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.create_greeting(request, context).await?;
    Ok(axum::Json(result))
}
pub async fn get_greeting<T, Cx>(
    State(handler): State<T>,
    context: Cx,
    request: GetGreetingRequest,
) -> Result<::axum::Json<Greeting>>
where
    T: GreetingHandler<Cx> + Clone + Send + Sync + 'static,
    Cx: axum::extract::FromRequestParts<T> + Send,
{
    let result = handler.get_greeting(request, context).await?;
    Ok(axum::Json(result))
}
