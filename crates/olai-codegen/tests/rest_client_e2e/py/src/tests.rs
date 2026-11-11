use axum::Router;
use axum::extract::{OriginalUri, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pyo3::prelude::*;
use serde_json::json;

async fn get_widget(Path(name): Path<String>, headers: axum::http::HeaderMap) -> Response {
    match name.as_str() {
        // The `Authorization` header, as the name.
        "token" => axum::Json(json!({
            "name": headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("none"),
        }))
        .into_response(),
        "missing" => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "error_code": "WIDGET_NOT_FOUND",
                "message": "no widget named missing",
                "details": [{"reason": "gone"}],
            })),
        )
            .into_response(),
        "green" => axum::Json(json!({"name": name, "color": "GREEN"})).into_response(),
        "json" => axum::Json(json!({
            "name": name,
            "attributes": {"count": 3, "ratio": 0.5, "nested": {"id": 7}},
            "extra": [1, "two", null, true],
            "notes": [{"a": 1}, 2.5],
            "rows": {"t": [1, 2]},
        }))
        .into_response(),
        _ => axum::Json(json!({
            "name": name,
            "color": "PURPLE",
            "created_at": 1_700_000_000_000_i64,
            "properties": {"a": "1"},
        }))
        .into_response(),
    }
}

async fn list_widgets(uri: OriginalUri) -> Response {
    let query = uri.query().unwrap_or_default();
    let page = if query.contains("color=GREEN") {
        json!({"widgets": [{"name": "g1", "color": "GREEN"}]})
    } else if query.contains("page_token=p2") {
        json!({"widgets": [{"name": "w3"}]})
    } else {
        json!({"widgets": [{"name": "w1"}, {"name": "w2"}], "next_page_token": "p2"})
    };
    axum::Json(page).into_response()
}

async fn create_widget(body: String) -> Response {
    let request: serde_json::Value = serde_json::from_str(&body).unwrap();
    // The raw body comes back as `display_name`, so the test can check its numbers.
    axum::Json(json!({
        "name": request["name"],
        "created_at": request["created_at"],
        "attributes": request["attributes"],
        "display_name": body,
        "type": request["type"],
        "labels_by_key": request["labels_by_key"],
        "notes": request["notes"],
    }))
    .into_response()
}

async fn widget_exists(Path(name): Path<String>) -> StatusCode {
    match name.as_str() {
        "missing" => StatusCode::NOT_FOUND,
        "denied" => StatusCode::FORBIDDEN,
        _ => StatusCode::OK,
    }
}

/// A `GET` with a JSON body.
async fn search_widgets(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let prefix = body["prefix"].as_str().unwrap_or_default().to_string();
    let widgets: Vec<_> = ["w1", "w2", "x1"]
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .map(|name| json!({"name": name}))
        .collect();
    axum::Json(json!({"widgets": widgets})).into_response()
}

/// An NDJSON answer.
async fn stream_widgets() -> Response {
    let lines: String = ["w1", "w2"]
        .into_iter()
        .map(|name| format!("{}\n", json!({"name": name})))
        .collect();
    (
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        lines,
    )
        .into_response()
}

/// Fields in the response headers.
async fn widget_version(Path(name): Path<String>) -> Response {
    let body = json!({"name": name}).to_string();
    ([("Widget-Version", "7"), ("ETag", "abc")], body).into_response()
}

/// Start the demo server on its own runtime, which must outlive the test.
fn serve() -> (String, tokio::runtime::Runtime) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let base = runtime.block_on(async {
        let app = Router::new()
            .route("/api/v1/widgets", get(list_widgets).post(create_widget))
            .route(
                "/api/v1/widgets/{name}",
                get(get_widget)
                    .delete(|| async { StatusCode::OK })
                    .head(widget_exists),
            )
            .route("/api/v1/search", get(search_widgets))
            .route("/api/v1/widget-lines", get(stream_widgets))
            .route("/api/v1/widgets/{name}/version", get(widget_version))
            .route(
                "/api/v1/blobs/{*path}",
                axum::routing::put(|body: axum::body::Bytes| async move {
                    assert_eq!(body.as_ref(), b"\x00\x01\xff");
                    StatusCode::NO_CONTENT
                })
                .get(
                    |Path(path): Path<String>, headers: axum::http::HeaderMap| async move {
                        match headers.get("Range").and_then(|v| v.to_str().ok()) {
                            Some(range) => format!("range:{range}"),
                            None => format!("blob:{path}"),
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/")
    });
    (base, runtime)
}

/// Run the Python test `file` of `tests/python/`, compiled into the test binary.
macro_rules! python_test {
    ($file:literal) => {
        run_python(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/python/", $file),
            include_str!(concat!("../tests/python/", $file)),
        )
    };
}

/// Run `code`, the Python file `path`, with `BASE` set to the server URL and the `demo`
/// package importable. Tracebacks name `path`.
fn run_python(path: &str, code: &str) {
    let (base, _server) = serve();
    Python::initialize();
    Python::attach(|py| {
        let result = (|| -> PyResult<()> {
            let sys = py.import("sys")?;
            let modules = sys.getattr("modules")?;
            if !modules.contains("demo._core")? {
                let core = pyo3::wrap_pymodule!(crate::codegen::extension_module)(py);
                let core = core.bind(py);
                modules.set_item("demo._core", core)?;
                crate::codegen::add_submodules_to_sys_modules(core)?;
            }
            sys.getattr("path")?
                .call_method1("insert", (0, crate::PACKAGE_DIR))?;
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("BASE", &base)?;
            globals.set_item("__file__", path)?;
            let builtins = py.import("builtins")?;
            let compiled = builtins.call_method1("compile", (code, path, "exec"))?;
            builtins.call_method1("exec", (compiled, globals))?;
            Ok(())
        })();
        if let Err(e) = result {
            let traceback = e
                .traceback(py)
                .and_then(|t| t.format().ok())
                .unwrap_or_default();
            panic!("{traceback}{e}");
        }
    });
}

#[cfg(feature = "cloud")]
#[test]
fn token_provider() {
    python_test!("test_token_provider.py");
}

#[test]
fn python_api() {
    python_test!("test_python_api.py");
}
