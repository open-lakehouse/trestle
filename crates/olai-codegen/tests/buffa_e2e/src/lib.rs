//! End-to-end proof that trestle's `Runtime::Buffa` codegen produces a client
//! that **compiles against real buffa-generated models** and that buffa's native
//! serde round-trips JSON with proto3-canonical semantics.
//!
//! The crate compiling at all is the primary assertion (the generated client in
//! `client::*` uses `buffa::EnumValue`, `buffa::MessageField`, `proto_name()`,
//! etc.); the test below adds the JSON round-trip check.

pub mod models {
    include!(concat!(env!("OUT_DIR"), "/models/_include.rs"));
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(String),
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("api: {0}")]
    Api(String),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.to_string())
    }
}

impl From<olai_http::Error> for Error {
    fn from(e: olai_http::Error) -> Self {
        Error::Http(e.to_string())
    }
}

pub mod error {
    use super::Error;
    pub async fn parse_error_response(resp: reqwest::Response) -> Error {
        Error::Api(format!("status {}", resp.status()))
    }
}

/// The trestle-generated client, emitted for `Runtime::Buffa` against the buffa
/// models above. If this `include!` fails to compile, the buffa codegen is wrong.
pub mod client {
    include!(concat!(env!("OUT_DIR"), "/client/mod.rs"));
}

#[cfg(test)]
mod tests {
    use crate::models::demo::v1::*;
    use buffa::EnumValue;

    #[test]
    fn buffa_request_roundtrips_json_with_enum_names() {
        let req = ListWidgetsRequest {
            max_results: 10,
            page_token: "tok".into(),
            color: EnumValue::Known(Color::Green),
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ListWidgetsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
        // proto3-canonical JSON: enums serialize as their proto NAME, not an int.
        assert!(json.contains("GREEN"), "enum should serialize as name: {json}");
    }

    /// Mirror the exact idioms the **node/NAPI** emitter generates for the buffa runtime, so a
    /// compile failure here flags a broken node codegen path:
    ///   - FFI `i32` -> bare enum via `<E as buffa::Enumeration>::from_i32`
    ///   - message body `Buffer` bytes -> message via `<M as buffa::Message>::decode_from_slice`
    #[test]
    fn buffa_node_marshalling_idioms_compile() {
        use buffa::{Enumeration as _, Message as _};

        // Required-enum NAPI param conversion (i32 -> bare enum, then builder wraps).
        let color = <Color as buffa::Enumeration>::from_i32(2).expect("known enum");
        assert_eq!(color, Color::Green);
        let _wrapped = EnumValue::Known(color);

        // Optional-enum setter conversion (Option<i32> -> Option<E>).
        let opt: Option<Color> = Some(1).and_then(<Color as buffa::Enumeration>::from_i32);
        assert_eq!(opt, Some(Color::Red));

        // Message-body decode from NAPI `Buffer` bytes.
        let bytes = Widget { name: "w".into(), color: EnumValue::Known(Color::Red), ..Default::default() }
            .encode_to_vec();
        let decoded = <Widget as buffa::Message>::decode_from_slice(&bytes).expect("decodes");
        assert_eq!(decoded.name, "w");
    }
}
