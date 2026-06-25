# buffa end-to-end fixture

Opt-in proof that trestle's `Runtime::Buffa` code generation produces a client
that **compiles against real [buffa](https://github.com/anthropics/buffa)
models** and that buffa's native serde round-trips JSON.

This crate is **excluded from the workspace** so `buffa`, `buffa-build`, and
`protoc` are not pulled into the default `cargo build` / `cargo test`. The
automated, in-CI regression coverage for buffa codegen output lives in the
`golden_buffa/` snapshot tree (see `tests/golden_integration.rs`); this fixture
is the heavier "does it actually compile and run" check.

## Scope: client, server, node, python

This fixture compiles the generated **client** (and exercises the **node/NAPI**
marshalling idioms) against real buffa models. The **Python (PyO3)** emitter is
runtime-invariant — it passes model enums/structs across the PyO3 boundary by
their bare type and lets the client builder apply the runtime's
`EnumValue`/`MessageField` wrapping, so it emits identical code for prost and
buffa (enforced by `python_and_ts_output_is_runtime_invariant` in the golden
test).

For those bare-type boundaries to compile, the model crate must carry
`FromPyObject`/`IntoPyObject` impls. trestle now emits these into the model
module (`models/_gen/pyo3_impls.rs`, gated on the `python` feature) via
`pythonize` over the models' serde derives — see `generate_pyo3_impls`. The
`python`-feature build of this fixture `include!`s that file and the
`buffa_model_roundtrips_through_pyo3` test proves it compiles against real buffa
types and round-trips a model (including an enum field, which crosses the
boundary as its proto name) Rust → Python → Rust:

```sh
cargo test --manifest-path crates/olai-codegen/tests/buffa_e2e/Cargo.toml --features python
```

## Running

Requires `protoc` on `PATH` (buffa-build invokes it). The proto sources and their
`google.api` dependencies are vendored under `proto/exported/` (via `buf export`)
and the descriptor under `proto/svc.bin`, so no network or `buf` is needed.

```sh
cargo test --manifest-path crates/olai-codegen/tests/buffa_e2e/Cargo.toml
```

## What it does

`build.rs` runs two generators into `OUT_DIR`:

1. **buffa-build** → buffa model types from `proto/exported/svc.proto`
   (`generate_json(true)`, no pbjson).
2. **olai-codegen** with `Runtime::Buffa` → the trestle HTTP client from
   `proto/svc.bin`.

`src/lib.rs` `include!`s both. The crate compiling is the main assertion (the
generated client uses `buffa::EnumValue::Known`, `buffa::MessageField::some`,
`Enumeration::proto_name`, …); the `#[test]`s add a JSON round-trip that checks
enums serialize as their proto name, plus a check that the **node/NAPI**
marshalling idioms (`Enumeration::from_i32` for FFI `i32` → enum, and
`Message::decode_from_slice` for `Buffer` bodies) compile against real buffa
types.

## Regenerating the vendored protos

```sh
cd proto && buf export <module> -o exported && buf build <module> -o svc.bin
```
