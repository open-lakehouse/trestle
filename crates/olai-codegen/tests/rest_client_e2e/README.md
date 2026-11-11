# REST client end-to-end fixture

Opt-in test that the REST client and Python options of `olai-codegen` generate code that
compiles, passes `clippy -D warnings`, and works against a real HTTP server.

From `proto/demo/v1/svc.proto`, with every option on (see `config.rs`), it generates:

- the buffa models with the `trestle` rewrites (snake_case JSON names, int64 as numbers);
- the Rust client with the generated `reqwest` transport and error module (`rest-client-e2e`);
  with the `cloud` feature, on `olai_http::CloudClient` (dual-transport) instead, with
  `token_provider=` and a default signer in the Python client;
- the Python bindings (`py/`, `rest-client-e2e-py`), tested in an embedded interpreter; built as
  an extension module, they are imported by a Python process and give the stubs that
  `pyo3-introspection` writes.

## Python tests

The tests of the Python bindings are the files of `py/tests/python/`. A `#[test]` in
`py/src/tests.rs` runs each one in the embedded interpreter (`python_test!`), with `BASE` set to
the URL of a test server and the generated `demo` package importable; a traceback names the file
and line. The files are compiled into the test binary, so a change to one reruns its test. Add a
file and a `python_test!` line for a new test. Check them with `ruff check` and `ruff format`.

## Running

Requires `protoc` on `PATH` and a Python with its shared library (e.g. Homebrew python3.12):

```sh
PYO3_PYTHON=/opt/homebrew/bin/python3.12 \
  cargo test --manifest-path crates/olai-codegen/tests/rest_client_e2e/Cargo.toml --workspace
```

With the `cloud` feature:

```sh
PYO3_PYTHON=/opt/homebrew/bin/python3.12 \
  cargo test --manifest-path crates/olai-codegen/tests/rest_client_e2e/Cargo.toml --workspace \
  --features cloud,rest-client-e2e-py/cloud
```
