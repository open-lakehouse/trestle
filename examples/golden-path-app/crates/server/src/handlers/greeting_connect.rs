//! Connect-RPC handler for the `Greeting` service.
//!
//! Implements the generated Connect `GreetingService` trait (in
//! `crate::connect_gen`) by delegating to the SAME [`GreetingCore`] the REST
//! handler uses. This is the seam that lets one binary serve both REST and
//! Connect on one port against one implementation: the two generated traits
//! differ in shape (owned request + `crate::api::Result` for REST; zero-copy
//! `ServiceRequest` + `ServiceResult`/`ConnectError` for Connect), but both
//! reduce to the same `core.create(...)` / `core.get(...)` calls.

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use golden_path_app_common::models::golden_path_app::v1::{
    CreateGreetingRequest, GetGreetingRequest, Greeting,
};

use crate::connect_gen::golden_path_app::v1::GreetingService;
use crate::handlers::core::{CoreError, GreetingCore};
use crate::handlers::greeting::Service;

/// Map domain errors onto the Connect error envelope.
impl From<CoreError> for ConnectError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::InvalidArgument(m) => ConnectError::invalid_argument(m),
            CoreError::NotFound(m) => ConnectError::not_found(m),
        }
    }
}

impl GreetingService for Service {
    async fn create_greeting(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateGreetingRequest>,
    ) -> ServiceResult<Greeting> {
        // Copy out of the zero-copy view before any await / move.
        let req = request.to_owned_message();
        let recipient = req
            .greeting
            .into_option()
            .map(|g| g.recipient)
            .ok_or_else(|| ConnectError::invalid_argument("greeting is required"))?;
        let core: &GreetingCore = &self.core();
        Response::ok(core.create(&recipient)?)
    }

    async fn get_greeting(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetGreetingRequest>,
    ) -> ServiceResult<Greeting> {
        let name = request.name.to_string();
        Response::ok(self.core().get(&name)?)
    }
}
