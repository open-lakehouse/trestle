# REST client end-to-end fixture

Opt-in test that the REST client and Python options of `olai-codegen` generate code that
compiles, passes `clippy -D warnings`, and works against a real HTTP server.

From `proto/demo/v1/svc.proto`, with every option on (see `config.rs`), it generates:

- the buffa models with the `trestle` rewrites (snake_case JSON names, int64 as numbers);
- the Rust client with the generated `reqwest` transport and error module (`rest-client-e2e`);
- the Python bindings (`py/`, `rest-client-e2e-py`), tested in an embedded interpreter; built as
  an extension module, they are imported by a Python process and give the stubs that
  `pyo3-introspection` writes.

## Running

Requires `protoc` on `PATH` and a Python with its shared library (e.g. Homebrew python3.12):

```sh
PYO3_PYTHON=/opt/homebrew/bin/python3.12 \
  cargo test --manifest-path crates/olai-codegen/tests/rest_client_e2e/Cargo.toml --workspace
```
