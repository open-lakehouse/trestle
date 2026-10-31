use std::ffi::CString;

use axum::Router;
use axum::extract::{OriginalUri, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pyo3::prelude::*;
use serde_json::json;

async fn get_widget(Path(name): Path<String>) -> Response {
    match name.as_str() {
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
            .route(
                "/api/v1/blobs/{*path}",
                get(|Path(path): Path<String>| async move { axum::Json(json!({"path": path})) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/")
    });
    (base, runtime)
}

/// Run `code` with `BASE` set to the server URL and the `demo` package importable.
fn run_python(code: &str) {
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
            py.run(&CString::new(code).unwrap(), Some(&globals), None)
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

#[test]
fn python_api() {
    run_python(
        r#"
import asyncio
import pickle

from demo._core import DemoClient, DemoError, NotFoundError, PermissionDeniedError, ServerError
from demo._core.demo import Label, Widget, WidgetClient
from demo._enums import Color

client = DemoClient(BASE)

# Module paths.
assert DemoClient.__module__ == "demo._core", DemoClient.__module__
assert Widget.__module__ == "demo._core.demo", Widget.__module__
assert WidgetClient.__module__ == "demo._core.demo", WidgetClient.__module__

# Frozen models with a Python repr; an unknown enum value reads as None.
w = client.widget("a/b c").get()
assert w.name == "a/b c"
assert w.color is None, w.color
assert w.created_at == 1700000000000
assert w.properties == {"a": "1"}
assert repr(w) == (
    "Widget(name='a/b c', created_at=1700000000000, size_bytes=0, "
    "properties={'a': '1'}, display_name='', metadata_location='', notes=[], rows={}, "
    "sizes=[], counts={}, labels_by_key={})"
), repr(w)
try:
    w.name = "x"
    raise AssertionError("models are frozen")
except AttributeError:
    pass

# Enum members are their names as str; parameters take a member or a name.
g = client.widget("green").get()
assert g.color is Color.GREEN
assert g.color == "GREEN" and isinstance(g.color, str) and str(g.color) == "GREEN"
assert f"{g.color}" == "GREEN"
assert [x.name for x in client.list_widgets(color="GREEN")] == ["g1"]
assert [x.name for x in client.list_widgets(color=Color.GREEN)] == ["g1"]
assert not hasattr(Color, "COLOR_UNSPECIFIED")
try:
    client.list_widgets(color="BLUE")
    raise AssertionError("unknown enum names are rejected")
except ValueError as e:
    assert "expected one of: RED, GREEN" in str(e), e

# Lists read every page.
assert [x.name for x in client.list_widgets()] == ["w1", "w2", "w3"]

# Only input models have a constructor; optional arguments are keyword-only.
label = Label(key="k", value="v")
assert Label("k").key == "k"
# Value equality; a `debug_redact` field is hidden in the repr only.
assert Label("k") == Label("k") and Label("k") != Label("j")
hidden = Label("k", secret="s3cret")
assert hidden.secret == "s3cret" and "s3cret" not in repr(hidden), repr(hidden)
assert repr(hidden) == "Label(key='k', value='', secret='***')", repr(hidden)
try:
    Label(value="v")
    raise AssertionError("expected a TypeError for the missing key")
except TypeError:
    pass
assert label.key == "k"
try:
    Widget(name="x")
    raise AssertionError("a response model has no constructor")
except TypeError:
    pass
created = client.create_widget("n", created_at=5, labels=[label])
assert created.created_at == 5
try:
    client.create_widget("n", 5)
    raise AssertionError("optional arguments are keyword-only")
except TypeError:
    pass

# Error answers raise the exception class of their status.
try:
    client.widget("missing").get()
    raise AssertionError("expected an error")
except NotFoundError as e:
    assert isinstance(e, DemoError) and isinstance(e, Exception)
    assert type(e).__module__ == "demo._core", type(e).__module__
    assert e.code == "WIDGET_NOT_FOUND" and e.status == 404, (e.code, e.status)
    assert e.details == [{"reason": "gone"}], e.details
    assert str(e) == "WIDGET_NOT_FOUND: no widget named missing", str(e)
    assert repr(e) == "NotFoundError('no widget named missing', code='WIDGET_NOT_FOUND', status=404)", repr(e)
    # Pickle round-trip, as multiprocessing needs.
    again = pickle.loads(pickle.dumps(e))
    assert type(again) is NotFoundError and str(again) == str(e) and again.details == e.details
    assert e.args == ("no widget named missing",), e.args
# Users can raise them too.
try:
    raise ServerError("down", status=503)
except DemoError as e:
    assert str(e) == "HTTP 503: down" and e.details == [], (str(e), e.details)
    assert e.args == ("down",), e.args
try:
    DemoClient("http://127.0.0.1:9/").widget("w").get()
    raise AssertionError("expected an error")
except DemoError as e:
    assert type(e) is DemoError and e.status is None and e.code is None
    # The message includes the Rust cause chain.
    assert "onnect" in str(e).split("error sending request", 1)[1], str(e)

# Free-form JSON fields are plain Python objects; whole numbers are ints both ways.
w = client.widget("json").get()
assert w.attributes == {"count": 3, "ratio": 0.5, "nested": {"id": 7}}, w.attributes
assert type(w.attributes["count"]) is int
assert w.extra == [1, "two", None, True], w.extra
assert w.notes == [{"a": 1}, 2.5], w.notes
assert w.rows == {"t": [1, 2]}, w.rows
created = client.create_widget("w2", attributes={"count": 3, "flag": True, "list": [1, 2.5]})
assert created.attributes == {"count": 3, "flag": True, "list": [1, 2.5]}, created.attributes
assert '"count":3' in created.display_name, created.display_name
# A field named after a Rust keyword.
assert client.create_widget("w4", type="gear").type == "gear"
# A map of messages.
by_key = client.create_widget("w5", labels_by_key={"k": label}).labels_by_key
assert list(by_key) == ["k"] and by_key["k"].value == "v", by_key
# A list of free-form JSON.
assert client.create_widget("w6", notes=[1, "two"]).notes == [1, "two"]
try:
    client.create_widget("w3", attributes={"bad": object()})
    raise AssertionError("expected a TypeError")
except TypeError:
    pass
# JSON has no NaN or infinity.
for bad in (float("nan"), float("inf")):
    try:
        client.create_widget("w3", attributes={"bad": bad})
        raise AssertionError("expected a ValueError")
    except ValueError:
        pass

# A HEAD existence check, and a GET with a JSON body.
assert client.widget_exists("w") is True
assert client.widget_exists("missing") is False
try:
    client.widget_exists("denied")
    raise AssertionError("expected an error")
except PermissionDeniedError:
    pass
assert [w.name for w in client.search_widgets("w", ["a"], {"t": [1, 2]}).widgets] == ["w1", "w2"]

# A multi-segment path value keeps its `/`.
assert client.get_blob("x/y").path == "x/y"

# Each method has an `_async` twin.
async def main():
    w = await client.widget("w").get_async()
    assert w.name == "w"
    assert await client.widget("w").delete_async() is None
    names = [x.name for x in await client.list_widgets_async()]
    assert names == ["w1", "w2", "w3"], names
    try:
        await client.widget("missing").get_async()
        raise AssertionError("expected an error")
    except NotFoundError:
        pass

asyncio.run(main())
"#,
    );
}
