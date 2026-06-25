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
    pub mod pyo3_impls {
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

    /// The PyO3 wrapper `#[pyclass]` types trestle emits (`pyo3_impls.rs`) must
    /// compile against real buffa models AND expose them as *real Python objects*:
    /// native attribute access, a keyword constructor, real enum members, and
    /// `From`/`Into` bridges to the bare model type so the generated client
    /// bindings can convert at the boundary.
    ///
    /// This is the gap the wrapper approach closes over the earlier pythonize
    /// impls, which only ever produced plain Python dicts.
    ///
    /// The enum field exercises the wrapper-enum bridge end to end: the buffa field
    /// is `EnumValue<Color>`, the wrapper getter yields a `PyColor` enum member, and
    /// the `From`/`Into` impls map through `Color`.
    #[cfg(feature = "python")]
    #[test]
    fn buffa_model_wraps_as_real_pyclass() {
        use crate::models::pyo3_impls::{PyColor, PyWidget};
        use pyo3::prelude::*;
        use pyo3::types::{PyAnyMethods, PyType};

        Python::initialize();
        Python::attach(|py| {
            // Construct via the bare model, then wrap.
            let widget = Widget {
                name: "gizmo".into(),
                color: EnumValue::Known(Color::Green),
                ..Default::default()
            };
            let wrapper: PyWidget = widget.clone().into();

            let obj = wrapper.clone().into_pyobject(py).expect("wrapper -> py object");

            // `isinstance(obj, Widget)` — a real class, not a dict.
            let widget_cls = obj.get_type();
            assert_eq!(
                widget_cls.name().unwrap().to_string(),
                "Widget",
                "wrapper is exposed as the `Widget` class"
            );

            // Native attribute access: `obj.name` returns the value, not a dict key.
            let name: String = obj.getattr("name").unwrap().extract().unwrap();
            assert_eq!(name, "gizmo");

            // The enum field is a real Python enum member (`Color.GREEN`), and it is
            // an instance of the `Color` pyclass enum.
            let color = obj.getattr("color").unwrap();
            let color_cls = color.get_type();
            assert_eq!(color_cls.name().unwrap().to_string(), "Color");
            let color_back: PyColor = color.extract().unwrap();
            assert_eq!(color_back, PyColor::GREEN);

            // `From`/`Into` bridge round-trips the data back to the bare model.
            let back: Widget = wrapper.into();
            assert_eq!(widget, back);

            // The wrapper type registers as a Python class.
            let _ = PyType::new::<PyWidget>(py);
        });
    }
}
