//! Server-wide error type. Implements `IntoResponse` so handlers return a uniform
//! shape (HTTP status + JSON `{ "error": { "code", "message" } }`).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub type Result<T> = std::result::Result<T, Error>;

// `#[allow(dead_code)]`: the starter handler only constructs a couple of these
// variants; the rest are the standard set real handlers reach for. Kept so the
// scaffold compiles under `-D warnings`.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum Error {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Unauthorized(String),

    #[error("internal: {0}")]
    Internal(String),
}

impl Error {
    fn status(&self) -> StatusCode {
        match self {
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Error::BadRequest(_) => "INVALID_ARGUMENT",
            Error::NotFound(_) => "NOT_FOUND",
            Error::Unauthorized(_) => "UNAUTHENTICATED",
            Error::Internal(_) => "INTERNAL",
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        (self.status(), Json(body)).into_response()
    }
}