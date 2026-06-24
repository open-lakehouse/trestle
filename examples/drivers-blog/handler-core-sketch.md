# Handler core sketch — implement once, serve twice

> SKETCH for the blog. The shapes below are lifted directly from the working
> `examples/golden-path-app/crates/server/src/handlers/` (just renamed to the
> driver domain). They are the pattern the dual-protocol golden path proved; a
> later session wires the real thing.

The generated code gives you, per service:

- a **handler trait** (`DriverHandler`) with one `async fn` per RPC — for REST;
- the **Axum route fns** that call it;
- a **Connect service trait** (`DriverService`) — for buf ConnectRPC;
- the typed Rust client + the browser WASM client.

You implement the business logic **once**, in a protocol-neutral core, then add
two thin adapters. Nothing about the domain logic knows whether the request
arrived over REST or Connect.

## 1. The core — protocol-neutral domain logic

```rust
// handlers/core.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use caspers_drivers_common::models::caspers::drivers::v1::{Driver, DriverStatus};

/// Domain errors, independent of HTTP/Connect. Each adapter maps these into its
/// own envelope.
#[derive(Debug)]
pub enum CoreError {
    InvalidArgument(String),
    NotFound(String),
}
pub type CoreResult<T> = Result<T, CoreError>;

/// In-memory driver roster. Swap the map for Postgres / Unity Catalog later —
/// the adapters don't change.
#[derive(Default, Clone)]
pub struct DriverCore {
    store: Arc<Mutex<HashMap<String, Driver>>>,
}

impl DriverCore {
    /// A driver checks in: assign the resource name, mark AVAILABLE.
    pub fn check_in(&self, display_name: &str, vehicle: &str) -> CoreResult<Driver> {
        if display_name.is_empty() {
            return Err(CoreError::InvalidArgument("display_name is required".into()));
        }
        let name = format!("drivers/{}", uuid::Uuid::new_v4());
        let driver = Driver {
            name: name.clone(),
            display_name: display_name.to_string(),
            vehicle: vehicle.to_string(),
            status: DriverStatus::Available.into(),
            ..Default::default()
        };
        self.store.lock().unwrap().insert(name, driver.clone());
        Ok(driver)
    }

    pub fn get(&self, name: &str) -> CoreResult<Driver> {
        self.store.lock().unwrap().get(name).cloned()
            .ok_or_else(|| CoreError::NotFound(format!("driver `{name}` not found")))
    }

    /// State transition: the driver checks out (status → OFF).
    pub fn check_out(&self, name: &str) -> CoreResult<Driver> {
        let mut store = self.store.lock().unwrap();
        let driver = store.get_mut(name)
            .ok_or_else(|| CoreError::NotFound(format!("driver `{name}` not found")))?;
        driver.status = DriverStatus::Off.into();
        Ok(driver.clone())
    }
}
```

## 2. REST adapter — implements the generated `DriverHandler`

```rust
// handlers/driver.rs
#[derive(Default, Clone)]
pub struct Service { core: DriverCore }
impl Service { pub fn core(&self) -> DriverCore { self.core.clone() } }

// Map domain errors → the REST error envelope.
impl From<CoreError> for crate::api::Error {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::InvalidArgument(m) => crate::api::Error::BadRequest(m),
            CoreError::NotFound(m)        => crate::api::Error::NotFound(m),
        }
    }
}

#[async_trait]
impl DriverHandler for Service {
    async fn check_in(&self, req: CheckInRequest, _cx: RequestContext) -> Result<Driver> {
        let d = req.driver.into_option()
            .ok_or_else(|| crate::api::Error::BadRequest("driver is required".into()))?;
        Ok(self.core.check_in(&d.display_name, &d.vehicle)?)
    }
    async fn get_driver(&self, req: GetDriverRequest, _cx: RequestContext) -> Result<Driver> {
        Ok(self.core.get(&req.name)?)
    }
    async fn check_out(&self, req: CheckOutRequest, _cx: RequestContext) -> Result<Driver> {
        Ok(self.core.check_out(&req.name)?)
    }
    // list_drivers … delegates to the core the same way.
}
```

## 3. Connect adapter — implements the generated `DriverService`, SAME core

```rust
// handlers/driver_connect.rs
// Map domain errors → the Connect envelope.
impl From<CoreError> for connectrpc::ConnectError { /* invalid_argument / not_found */ }

#[allow(refining_impl_trait)]  // async fn -> ServiceResult<T> narrows the trait's impl-Future
impl DriverService for Service {
    async fn check_in(&self, _ctx: RequestContext, req: ServiceRequest<'_, CheckInRequest>)
        -> ServiceResult<Driver>
    {
        let req = req.to_owned_message();                 // copy out of the zero-copy view
        let d = req.driver.into_option()
            .ok_or_else(|| ConnectError::invalid_argument("driver is required"))?;
        Response::ok(self.core().check_in(&d.display_name, &d.vehicle)?)
    }
    // get_driver / check_out … same delegation.
}
```

Both traits differ in shape (owned request + `api::Result` for REST; zero-copy
`ServiceRequest` + `ServiceResult`/`ConnectError` for Connect) — but each method
is one line of translation around the **same** `self.core()` call.

## 4. Serve both on one port

```rust
// main.rs
fn build_router() -> Router {
    let svc = handlers::driver::Service::new();   // clones share one DriverCore

    // REST: the generated Axum route fns, with `Service` as state.
    let rest = Router::new()
        .route("/v1/drivers", post(create_driver::<Service, _>))
        .route("/v1/{*name}", get(get_driver::<Service, _>))
        .with_state(svc.clone());

    // Connect: the generated facade router, mounted as the fallback service.
    let connect = DriverServiceExt::register(Arc::new(svc), connectrpc::Router::new());

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(rest)
        .fallback_service(connect.into_axum_service());  // one listener, both protocols
}
```

The payoff (proven in `golden-path-app`): check a driver in over Connect, read
the roster over REST — same in-memory core, one port.
