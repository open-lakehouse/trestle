// @generated — do not edit by hand.
#![allow(unused_mut)]
#[cfg(not(target_arch = "wasm32"))]
type BoxFut<'a, T> = ::futures::future::BoxFuture<'a, T>;
#[cfg(target_arch = "wasm32")]
type BoxFut<'a, T> = ::futures::future::LocalBoxFuture<'a, T>;
use super::client::*;
use crate::api::Result;
use golden_path_app_common::models::golden_path_app::v1::*;
use std::future::IntoFuture;
/// Builder for creating a greeting
pub struct CreateGreetingBuilder {
    client: GreetingServiceClient,
    request: CreateGreetingRequest,
}
impl CreateGreetingBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `GreetingServiceClient`.
    pub(crate) fn new(client: GreetingServiceClient, greeting: Greeting) -> Self {
        let request = CreateGreetingRequest {
            greeting: buffa::MessageField::some(greeting),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for CreateGreetingBuilder {
    type Output = Result<Greeting>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.create_greeting(&request).await })
    }
}
/// Builder for getting a greeting
pub struct GetGreetingBuilder {
    client: GreetingServiceClient,
    request: GetGreetingRequest,
}
impl GetGreetingBuilder {
    /// Create a new builder instance.
    /// Obtain via the corresponding method on `GreetingServiceClient`.
    pub(crate) fn new(client: GreetingServiceClient, name: impl Into<String>) -> Self {
        let request = GetGreetingRequest {
            name: name.into(),
            ..Default::default()
        };
        Self { client, request }
    }
}
impl IntoFuture for GetGreetingBuilder {
    type Output = Result<Greeting>;
    type IntoFuture = BoxFut<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        let client = self.client;
        let request = self.request;
        Box::pin(async move { client.get_greeting(&request).await })
    }
}
