//! REST handler for the starter `Greeting` service.
//!
//! Implements the generated `GreetingHandler` trait (in `crate::codegen::greeting`)
//! by delegating to the protocol-agnostic [`GreetingCore`]. The Connect handler
//! (`handlers::greeting_connect`) delegates to the SAME core, so both transports
//! share one implementation.
//!
//! To add an RPC: edit `proto/golden_path_app/v1/service.proto`,
//! run `just regen` (regenerates the trait + route fns), then add the method
//! here and mount its route in `main.rs`.
//!
//! NOTE: this references generated code, so run `just regen` before the first
//! `cargo build` on a freshly-scaffolded tree.

use async_trait::async_trait;
use golden_path_app_common::models::golden_path_app::v1::*;

use crate::api::{Error, RequestContext, Result};
use crate::codegen::greeting::GreetingHandler;
use crate::handlers::core::{CoreError, GreetingCore};

/// REST-facing service: a thin wrapper over the shared [`GreetingCore`].
#[derive(Default, Clone)]
pub struct Service {
    core: GreetingCore,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    /// Expose the shared core so `main.rs` can hand the same instance to the
    /// Connect handler (so both transports see the same data).
    pub fn core(&self) -> GreetingCore {
        self.core.clone()
    }
}

/// Map domain errors onto the REST error envelope.
impl From<CoreError> for Error {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::InvalidArgument(m) => Error::BadRequest(m),
            CoreError::NotFound(m) => Error::NotFound(m),
        }
    }
}

#[async_trait]
impl GreetingHandler for Service {
    async fn create_greeting(
        &self,
        request: CreateGreetingRequest,
        _context: RequestContext,
    ) -> Result<Greeting> {
        let input = request
            .greeting
            .into_option()
            .ok_or_else(|| Error::BadRequest("greeting is required".into()))?;
        Ok(self.core.create(&input.recipient)?)
    }

    async fn get_greeting(
        &self,
        request: GetGreetingRequest,
        _context: RequestContext,
    ) -> Result<Greeting> {
        Ok(self.core.get(&request.name)?)
    }
}
