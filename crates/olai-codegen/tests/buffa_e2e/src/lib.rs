//! End-to-end proof that trestle's `Runtime::Buffa` codegen produces a client
//! that **compiles against real buffa-generated models** and that buffa's native
//! serde round-trips JSON with proto3-canonical semantics.
//!
//! The crate compiling at all is the primary assertion (the generated client in
//! `client::*` uses `buffa::EnumValue`, `buffa::MessageField`, `proto_name()`,
//! etc.); the test below adds the JSON round-trip check.

pub mod models {
    include!(concat!(env!("OUT_DIR"), "/models/_include.rs"));

    // PyO3 boundary conversions for the buffa model types, emitted by trestle into
    // the models output dir. The generated impls address models as
    // `super::demo::v1::Widget`, i.e. relative to the *models module's* parent —
    // exactly as they resolve in the real flow where trestle emits
    // `#[cfg(feature = "python")] mod pyo3_impls;` inside the models `mod.rs`. We
    // reproduce that here with a child `pyo3_impls` module so `super` is this
    // `models` module and `super::demo::v1::*` resolves to the buffa models above.
    #[cfg(feature = "python")]
    mod pyo3_impls {
        include!(concat!(env!("OUT_DIR"), "/models_gen/_gen/pyo3_impls.rs"));
    }
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
///
/// Named `codegen` because the generated aggregate root client (emitted once a
/// `bindings` config is present) refers to the per-service clients as
/// `crate::codegen::<service>` — the module layout trestle assumes for the client
/// crate.
pub mod codegen {
    include!(concat!(env!("OUT_DIR"), "/client/mod.rs"));
}

/// Back-compat alias: earlier the generated client was mounted at `crate::client`.
pub use codegen as client;

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

    /// The PyO3 boundary impls trestle emits (`pyo3_impls.rs`) must compile against
    /// real buffa models AND preserve data through a Rust → Python → Rust round
    /// trip. This is the gap the buffa/Python support closes: the generated
    /// bindings pass models across the FFI boundary by their bare type, which only
    /// works once these `IntoPyObject`/`FromPyObject` impls exist.
    ///
    /// The enum field exercises the proto-name-string contract end to end: buffa
    /// serializes `Color::Green` as `"GREEN"`, pythonize carries that string into
    /// Python and back, and `FromPyObject` reconstructs the `EnumValue::Known`.
    #[cfg(feature = "python")]
    #[test]
    fn buffa_model_roundtrips_through_pyo3() {
        use pyo3::prelude::*;
        use pyo3::types::PyAnyMethods;

        Python::initialize();
        Python::attach(|py| {
            let widget = Widget {
                name: "gizmo".into(),
                color: EnumValue::Known(Color::Green),
                ..Default::default()
            };

            // Rust → Python (IntoPyObject via pythonize).
            let obj = widget.clone().into_pyobject(py).expect("into_pyobject");
            // pythonize maps a proto message to a Python dict; the enum is its
            // proto name string.
            let color: String = obj
                .get_item("color")
                .expect("color key")
                .extract()
                .expect("color is a str");
            assert_eq!(color, "GREEN", "enum crosses the boundary as its proto name");

            // Python → Rust (FromPyObject via depythonize) round-trips the data.
            let back: Widget = obj.extract().expect("extract Widget");
            assert_eq!(widget, back);
        });
    }
}
