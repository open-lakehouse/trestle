//! HTTP client for the golden-path-app service.
//!
//! The generated client lives under `codegen` (populated by `trestle generate`)
//! and is re-exported from the crate root. The hand-written `api` and `error`
//! modules below provide the `Result` / `Error` types and the
//! `parse_error_response` helper the generated code calls — these are stable and
//! safe to edit; everything under `gen/` is overwritten on every regen.

pub use golden_path_app_common::models;

/// Client `Result` / `Error` types. The generated client returns
/// `crate::api::Result<T>` (see `result_type` in `trestle.yaml`).
pub mod api {
    pub type Result<T> = std::result::Result<T, Error>;

    /// Errors surfaced by the generated client.
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        /// Transport / HTTP error (request never produced a usable response).
        #[error("http: {0}")]
        Http(String),
        /// Failed to build a request URL.
        #[error("url: {0}")]
        Url(#[from] url::ParseError),
        /// Failed to (de)serialize a request/response body.
        #[error("json: {0}")]
        Json(#[from] serde_json::Error),
        /// The server returned a non-success status. Carries the decoded
        /// `{ code, message }` envelope when present.
        #[error("api[{code}]: {message}")]
        Api { code: String, message: String },
    }

    impl From<reqwest::Error> for Error {
        fn from(e: reqwest::Error) -> Self {
            Error::Http(e.to_string())
        }
    }

    // The cloud transport's error only exists off-wasm. On wasm32 the browser
    // transport surfaces `reqwest::Error` (handled above), so no extra impl.
    #[cfg(not(target_arch = "wasm32"))]
    impl From<olai_http::Error> for Error {
        fn from(e: olai_http::Error) -> Self {
            Error::Http(e.to_string())
        }
    }
}

pub use api::{Error, Result};

/// Decodes a non-success HTTP response into an [`api::Error`]. Generated client
/// methods call this on any non-2xx status.
pub mod error {
    use super::api::Error;

    /// Parse the server's `{ "error": { "code", "message" } }` envelope. Falls
    /// back to the raw status when the body is missing or not in that shape.
    pub async fn parse_error_response(resp: reqwest::Response) -> Error {
        let status = resp.status();
        match resp.bytes().await {
            Ok(body) => match serde_json::from_slice::<serde_json::Value>(&body) {
                Ok(v) => {
                    let err = v.get("error");
                    let code = err
                        .and_then(|e| e.get("code"))
                        .and_then(|c| c.as_str())
                        .unwrap_or(status.as_str())
                        .to_string();
                    let message = err
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("status {status}"));
                    Error::Api { code, message }
                }
                Err(_) => Error::Api {
                    code: status.as_str().to_string(),
                    message: format!("status {status}"),
                },
            },
            Err(e) => Error::Http(e.to_string()),
        }
    }
}

/// The generated client (typed per-service clients + builders + the aggregate
/// `GoldenPathAppClient`). Overwritten by `trestle generate`.
#[path = "gen/mod.rs"]
pub mod codegen;

pub use codegen::*;

/// `#[wasm_bindgen]` browser bindings (generated into `wasm/bindings.rs`). The
/// file self-gates on `cfg(target_arch = "wasm32")`, so a native build compiles
/// it away; `wasm-pack` (via `just build-wasm`) packages it into the frontend.
#[path = "wasm/bindings.rs"]
mod wasm_bindings;