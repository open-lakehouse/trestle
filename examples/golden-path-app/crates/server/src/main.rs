//! golden-path-app server entry point.
//!
//! Databricks Apps runs this binary in a container and routes traffic to the
//! port it sets via the `DATABRICKS_APP_PORT` env var (defaulting to 8080).
//! Headers like `X-Forwarded-Access-Token` carry the end-user's OBO credential.

use std::env;
use std::net::SocketAddr;

use axum::Router;
use axum::routing::{get, post};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

mod api;
// `gen` is a reserved keyword in Rust 2024, so the generated `gen/` directory is
// mounted under the `codegen` alias (matching the client crate).
#[path = "gen/mod.rs"]
mod codegen;
// Generated Connect-RPC service facade (buffa views + connectrpc traits).
#[path = "connect_gen/mod.rs"]
mod connect_gen;
mod handlers;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let port: u16 = env::var("DATABRICKS_APP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();

    let app = build_router();

    tracing::info!(%addr, "golden-path-app server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router() -> Router {
    use std::sync::Arc;

    use handlers::greeting::Service;

    // One service instance backing BOTH transports. `Service` clones share the
    // same underlying `GreetingCore` (an `Arc`), so a create over REST is
    // visible to a get over Connect and vice-versa.
    let svc = Service::new();

    // --- REST surface (generated Axum route fns from the proto's google.api.http) ---
    use crate::codegen::greeting::server::{create_greeting, get_greeting};
    let rest_routes = Router::new()
        // POST /v1/greetings  (google.api.http: post "/v1/greetings", body "greeting")
        .route(
            "/v1/greetings",
            post(create_greeting::<Service, api::RequestContext>),
        )
        // GET  /v1/greetings/{uuid}  (google.api.http: get "/v1/{name=greetings/*}").
        // Wildcard so the captured `name` keeps the `greetings/` prefix.
        .route("/v1/{*name}", get(get_greeting::<Service, api::RequestContext>))
        .with_state(svc.clone());

    // --- Connect surface (generated connectrpc facade; POSTs to
    //     /golden_path_app.v1.GreetingService/<Method>) ---
    use crate::connect_gen::golden_path_app::v1::GreetingServiceExt;
    let connect_router = GreetingServiceExt::register(Arc::new(svc), connectrpc::Router::new());

    // Both on one listener: REST routes are explicit; everything else (the
    // Connect RPC paths) falls through to the Connect service.
    Router::new()
        // Liveness/readiness for Databricks Apps and CI smoke tests.
        .route("/healthz", get(|| async { "ok" }))
        .merge(rest_routes)
        .fallback_service(connect_router.into_axum_service())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("APP_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}