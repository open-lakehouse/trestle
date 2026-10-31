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
marshalling idioms) against real buffa models.

The **Python (PyO3)** emitter wraps each model message in a frozen newtype
`#[pyclass]` (`PyWidget(Widget)`) with native typed getters, a keyword constructor
on the models that a request takes, a Python-style `repr`, and `From`/`Into`
bridges to the bare model type — so a model is a *real Python object* (attribute
access, `isinstance`), not a plain dict. Enums are `enum.StrEnum` classes in the
generated Python package (`python_pkg/demo/_enums.py` under `OUT_DIR`): the enum
getters return their members, and a parameter takes a member or its name. The
wrapper *bodies* are runtime-aware (buffa `EnumValue<E>` / `MessageField<T>` vs
prost `i32` / `Option<Box<T>>`); the public surface (class + method names +
Python-visible types) is identical across runtimes.

trestle emits the wrappers into the model module
(`models/_gen/pyo3_impls.rs`, gated on the `python` feature) — see
`generate_pyo3_impls`. The `python`-feature build of this fixture `include!`s that
file and the `buffa_model_wraps_as_real_pyclass` test proves it compiles against
real buffa types and that a model wraps as a real Python object — native
attribute access, an `isinstance`-checkable class, a `StrEnum` member, and a
`From`/`Into` round-trip back to the bare model. `enum.StrEnum` needs Python
3.11+, so point PyO3 at one:

```sh
PYO3_PYTHON=/opt/homebrew/bin/python3.12 \
  cargo test --manifest-path crates/olai-codegen/tests/buffa_e2e/Cargo.toml --features python
```

The generated PyO3 **client bindings** are not compiled here; `tests/rest_client_e2e`
builds them and drives them from Python against a server.

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
2. **olai-codegen** with `Runtime::Buffa` → the trestle HTTP client, with its
   error module, from `proto/svc.bin`.

`src/lib.rs` `include!`s both. The crate compiling is the main assertion (the
generated client uses `buffa::EnumValue::Known`, `buffa::MessageField::some`,
`Enumeration::proto_name`, …); the `#[test]`s add a JSON round-trip that checks
enums serialize as their proto name, plus a check that the **node/NAPI**
marshalling idioms (`Enumeration::from_i32` for FFI `i32` → enum, and
`Message::decode_from_slice` for `Buffer` bodies) compile against real buffa
types.

## Regenerating the vendored protos

```sh
cd proto && buf export <module> -o exported
protoc -I exported --include_imports --descriptor_set_out=svc.bin exported/svc.proto
```

Or, after editing `proto/exported/svc.proto` in place:

```sh
protoc -I proto/exported --include_imports --descriptor_set_out=proto/svc.bin proto/exported/svc.proto
```
