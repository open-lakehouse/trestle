//! The trestle-generated PyO3 client bindings for the buffa e2e fixture, compiled for real.
//!
//! The bindings (`codegen/`) are emitted by `buffa-e2e`'s build script and `include!`d here,
//! laid out exactly as in a real Python binding crate: the generated `codegen` module, a
//! hand-written `error` module supplying the configured `PyDemoError`, and the models
//! re-exported at `crate::models`. Compiling this crate is the primary assertion; the tests
//! then drive the sync and async clients from Python against a local HTTP stub.

use pyo3::prelude::*;

pub use buffa_e2e::models;

/// The hand-written error module the generated bindings expect at `crate::error`.
pub mod error {
    use pyo3::PyErr;
    use pyo3::create_exception;
    use pyo3::exceptions::PyException;

    create_exception!(demo, DemoError, PyException);

    /// The configured `py_error_type`: client errors map into it, and it maps into `PyErr`.
    #[derive(Debug)]
    pub struct PyDemoError(String);

    impl From<buffa_e2e::Error> for PyDemoError {
        fn from(e: buffa_e2e::Error) -> Self {
            Self(e.to_string())
        }
    }

    impl From<url::ParseError> for PyDemoError {
        fn from(e: url::ParseError) -> Self {
            Self(e.to_string())
        }
    }

    impl From<PyDemoError> for PyErr {
        fn from(e: PyDemoError) -> Self {
            DemoError::new_err(e.0)
        }
    }
}

// `pub mod codegen;`, mounted from the generated dir (see `build.rs`).
include!(concat!(env!("OUT_DIR"), "/codegen.rs"));

/// The extension module, as a consumer would write it.
#[pymodule]
fn demo(m: &Bound<'_, PyModule>) -> PyResult<()> {
    codegen::register_clients(m)?;
    m.add("DemoError", m.py().get_type::<error::DemoError>())
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    use pyo3::prelude::*;
    use pyo3::types::PyDict;

    /// Serve `route` on a local port, one thread per connection; returns the base URL.
    fn serve(route: fn(&str) -> (&'static str, String)) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || respond(stream, route));
            }
        });
        format!("http://{addr}/")
    }

    /// Canned JSON for the `GadgetService` routes. `GET /gadgets` answers with two pages (`a`,
    /// `b` then `c`) so the pagers must follow `next_page_token`; `GET /gadgets/{name}` echoes
    /// the gadget back.
    fn gadgets(target: &str) -> (&'static str, String) {
        if target == "/gadgets" || target.starts_with("/gadgets?") {
            if target.contains("page_token=p2") {
                ("200 OK", r#"{"gadgets":[{"name":"c"}]}"#.to_string())
            } else {
                (
                    "200 OK",
                    r#"{"gadgets":[{"name":"a"},{"name":"b"}],"nextPageToken":"p2"}"#.to_string(),
                )
            }
        } else if let Some(name) = target.strip_prefix("/gadgets/") {
            ("200 OK", format!(r#"{{"name":"{name}","color":"RED"}}"#))
        } else {
            ("404 Not Found", "{}".to_string())
        }
    }

    /// Every request fails with a (non-retried) 404, to check error mapping.
    fn not_found(_target: &str) -> (&'static str, String) {
        ("404 Not Found", "{}".to_string())
    }

    fn respond(mut stream: TcpStream, route: fn(&str) -> (&'static str, String)) {
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => head.extend_from_slice(&chunk[..n]),
            }
        }
        let head = String::from_utf8_lossy(&head);
        let target = head.split_whitespace().nth(1).unwrap_or("/");
        let (status, body) = route(target);
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
    }

    /// A server that accepts connections but never answers, for cancellation tests.
    fn serve_nothing() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let held: Vec<TcpStream> = listener.incoming().flatten().collect();
            drop(held);
        });
        format!("http://{addr}/")
    }

    /// Run `code` with the extension module bound as `demo` and `base` / `broken` / `hang` set to
    /// the stub URLs, printing the Python traceback on failure.
    fn run_python(code: &str) {
        Python::initialize();
        let base = serve(gadgets);
        let broken = serve(not_found);
        let hang = serve_nothing();
        Python::attach(|py| {
            let module = pyo3::wrap_pymodule!(super::demo)(py);
            let globals = PyDict::new(py);
            globals.set_item("demo", module).unwrap();
            globals.set_item("base", base).unwrap();
            globals.set_item("hang", hang).unwrap();
            globals.set_item("broken", broken).unwrap();
            let code = CString::new(code).unwrap();
            if let Err(err) = py.run(&code, Some(&globals), None) {
                err.print(py);
                panic!("python test failed: {err}");
            }
        });
    }

    #[test]
    fn sync_client_calls_and_paginates() {
        run_python(
            r#"
client = demo.DemoClient(base)
assert client.gadget("x").get().name == "x"
client.gadget("x").delete()

# Lists return a lazy pager that follows `next_page_token` across pages.
pager = client.list_gadgets()
assert iter(pager) is pager
assert [g.name for g in pager] == ["a", "b", "c"]
assert list(pager) == []  # exhausted

# API errors surface as the configured error type, for calls and pagers alike.
bad = demo.DemoClient(broken)
for call in (lambda: bad.gadget("x").get(), lambda: next(bad.list_gadgets())):
    try:
        call()
    except demo.DemoError:
        pass
    else:
        raise AssertionError("expected DemoError")
"#,
        );
    }

    #[test]
    fn async_client_awaits_paginates_and_cancels() {
        run_python(
            r#"
import asyncio

async def main():
    client = demo.AsyncDemoClient(base)
    gadget = await client.gadget("x").get()
    assert gadget.name == "x"
    assert await client.gadget("x").delete() is None

    # Concurrent awaits share the one runtime.
    names = await asyncio.gather(*(client.gadget(n).get() for n in "pqr"))
    assert [g.name for g in names] == ["p", "q", "r"]

    pager = client.list_gadgets()
    assert pager.__aiter__() is pager
    assert [g.name async for g in pager] == ["a", "b", "c"]

    bad = demo.AsyncDemoClient(broken)
    try:
        await bad.gadget("x").get()
    except demo.DemoError:
        pass
    else:
        raise AssertionError("expected DemoError")
    try:
        async for _ in bad.list_gadgets():
            pass
    except demo.DemoError:
        pass
    else:
        raise AssertionError("expected DemoError")

    # Cancelling the awaitable (here via a timeout) drops the in-flight request.
    try:
        await asyncio.wait_for(demo.AsyncDemoClient(hang).gadget("x").get(), 0.2)
    except asyncio.TimeoutError:
        pass
    else:
        raise AssertionError("expected a timeout")

asyncio.run(main())
"#,
        );
    }

    /// The sync and async clients run on one runtime: the one `pyo3-async-runtimes` owns.
    #[test]
    fn sync_and_async_share_the_runtime() {
        assert!(std::ptr::eq(
            crate::codegen::runtime::runtime(),
            pyo3_async_runtimes::tokio::get_runtime()
        ));
    }
}
