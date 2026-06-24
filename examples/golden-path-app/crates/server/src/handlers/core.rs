//! The business core — protocol-agnostic domain logic for greetings.
//!
//! Both the REST handler (`handlers::greeting`) and the Connect handler
//! (`handlers::greeting_connect`) delegate into this one type, so the actual
//! behaviour lives in exactly one place regardless of how the request arrived.
//! Swap the in-memory map for a real backend (Postgres, Unity Catalog, …) and
//! both transports follow.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use golden_path_app_common::models::golden_path_app::v1::Greeting;

/// Domain-level errors, independent of HTTP or Connect. Each transport maps
/// these into its own error envelope (`api::Error` for REST, `ConnectError`
/// for Connect).
#[derive(Debug)]
pub enum CoreError {
    InvalidArgument(String),
    NotFound(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

/// In-memory greeting store, keyed by resource name (`greetings/{uuid}`).
/// Cheaply cloneable (shared `Arc`) so it can back multiple handler structs.
#[derive(Default, Clone)]
pub struct GreetingCore {
    store: Arc<Mutex<HashMap<String, Greeting>>>,
}

impl GreetingCore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a greeting for `recipient`, assigning the resource name and the
    /// rendered (OUTPUT_ONLY) message.
    pub fn create(&self, recipient: &str) -> CoreResult<Greeting> {
        if recipient.is_empty() {
            return Err(CoreError::InvalidArgument("recipient is required".into()));
        }
        let name = format!("greetings/{}", uuid::Uuid::new_v4());
        let greeting = Greeting {
            name: name.clone(),
            recipient: recipient.to_string(),
            message: format!("hello, {recipient}!"),
            ..Default::default()
        };
        self.store.lock().unwrap().insert(name, greeting.clone());
        Ok(greeting)
    }

    /// Fetch a greeting by resource name.
    pub fn get(&self, name: &str) -> CoreResult<Greeting> {
        self.store
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("greeting `{name}` not found")))
    }
}
