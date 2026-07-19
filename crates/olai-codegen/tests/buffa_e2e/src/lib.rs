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
        use crate::models::demo::v1::widget::Size;
        use crate::models::pyo3_impls::{PyColor, PyWidget, PyWidgetSize};
        use pyo3::prelude::*;
        use pyo3::types::{PyAnyMethods, PyType};

        Python::initialize();
        Python::attach(|py| {
            // Construct via the bare model, then wrap. `size` is a *message-nested*
            // enum (`widget::Size`) — its wrapper must resolve through the snake_case
            // parent module, the case the buffa nested-enum path fix addresses.
            let widget = Widget {
                name: "gizmo".into(),
                color: EnumValue::Known(Color::Green),
                size: EnumValue::Known(Size::LARGE),
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

            // The *nested* enum field is exposed as a real Python enum member. Its
            // class is parent-qualified (`WidgetSize`) to stay collision-free, and
            // resolving it proves the snake_case parent-module path works end to end.
            let size = obj.getattr("size").unwrap();
            assert_eq!(size.get_type().name().unwrap().to_string(), "WidgetSize");
            let size_back: PyWidgetSize = size.extract().unwrap();
            assert_eq!(size_back, PyWidgetSize::LARGE);

            // `From`/`Into` bridge round-trips the data (incl. the nested enum) back to
            // the bare model.
            let back: Widget = wrapper.into();
            assert_eq!(widget, back);

            // The wrapper types register as Python classes.
            let _ = PyType::new::<PyWidget>(py);
            let _ = PyType::new::<PyWidgetSize>(py);
        });
    }

    /// The PyO3 wrapper `#[new]` constructor must be *oneof-aware* (issue #99):
    /// a message with a `oneof` exposes one optional keyword arg per variant, per-
    /// variant getters, and per-variant setters. Setting one variant sets the
    /// corresponding oneof on the real buffa model; the pyo3 methods are only
    /// callable through the interpreter, so this drives them via Python — which
    /// also proves the flattened kwargs reach `#[new]` and the getters round-trip.
    ///
    /// Exercises both the boxed-message variant path (buffa boxes every message
    /// oneof variant, e.g. `Shape::Circle(Box<Circle>)`) and the scalar variant
    /// path (`Shape::Label(String)`) against real buffa types.
    #[cfg(feature = "python")]
    #[test]
    fn buffa_oneof_wraps_as_flattened_pyclass_ctor() {
        use crate::models::demo::v1::shape::Shape as ShapeOneof;
        use crate::models::pyo3_impls::{PyCircle, PyShape, PySquare};
        use pyo3::prelude::*;
        use pyo3::types::{PyAnyMethods, PyType};

        Python::initialize();
        Python::attach(|py| {
            let shape_cls = PyType::new::<PyShape>(py);

            // Construct via the flattened message-variant kwarg: `Shape(circle=...)`.
            let circle_obj = PyCircle::from(Circle { radius: 2.5, ..Default::default() })
                .into_pyobject(py)
                .unwrap();
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("circle", circle_obj).unwrap();
            let shape = shape_cls.call((), Some(&kwargs)).unwrap();

            // The active variant reads back through its getter; the others are `None`.
            assert!(!shape.getattr("circle").unwrap().is_none(), "circle is active");
            assert!(shape.getattr("square").unwrap().is_none(), "square inactive");
            assert!(shape.getattr("label").unwrap().is_none(), "label inactive");
            let circle_back: PyCircle = shape.getattr("circle").unwrap().extract().unwrap();
            assert_eq!(Circle::from(circle_back).radius, 2.5);

            // The bare buffa model carries the *boxed* message variant.
            let wrapper: PyShape = shape.extract().unwrap();
            let bare: crate::models::demo::v1::Shape = wrapper.into();
            match bare.shape {
                Some(ShapeOneof::Circle(c)) => assert_eq!(c.radius, 2.5),
                other => panic!("expected boxed Circle variant, got {other:?}"),
            }

            // The scalar variant flattens as a plain keyword arg.
            let labeled_kwargs = pyo3::types::PyDict::new(py);
            labeled_kwargs.set_item("label", "hexagon").unwrap();
            let labeled = shape_cls.call((), Some(&labeled_kwargs)).unwrap();
            let label: String = labeled.getattr("label").unwrap().extract().unwrap();
            assert_eq!(label, "hexagon");
            assert!(labeled.getattr("circle").unwrap().is_none());

            // A later variant wins when more than one kwarg is supplied (declaration
            // order: circle, square, label — so `square` overrides `circle`).
            let square_obj = PySquare::from(Square { side: 3.0, ..Default::default() })
                .into_pyobject(py)
                .unwrap();
            let both_kwargs = pyo3::types::PyDict::new(py);
            both_kwargs
                .set_item("circle", PyCircle::from(Circle::default()).into_pyobject(py).unwrap())
                .unwrap();
            both_kwargs.set_item("square", square_obj).unwrap();
            let both = shape_cls.call((), Some(&both_kwargs)).unwrap();
            assert!(both.getattr("circle").unwrap().is_none(), "square overrides circle");
            let sq: PySquare = both.getattr("square").unwrap().extract().unwrap();
            assert_eq!(Square::from(sq).side, 3.0);

            // Per-variant setter round-trips through the oneof, and clearing the
            // *active* variant clears the oneof.
            let m = shape_cls.call((), None).unwrap();
            m.setattr("label", "octagon").unwrap();
            let set_label: String = m.getattr("label").unwrap().extract().unwrap();
            assert_eq!(set_label, "octagon");
            m.setattr("label", py.None()).unwrap();
            assert!(m.getattr("label").unwrap().is_none());
        });
    }
}
