//! WASM/browser binding generation: a `#[wasm_bindgen]` wrapper layer over the generated
//! (WASM-transport) clients, plus a `.d.ts` for JS/TS consumers.
//!
//! Unlike the NAPI path (which wraps a *native* Rust client and marshals protobuf bytes across the
//! addon boundary), the WASM path compiles the generated client *itself* to `wasm32` and exposes it
//! to JS via `wasm-bindgen`. Request/response values cross as plain JS objects through
//! `serde-wasm-bindgen`, so JS callers work with typed objects directly — no `Buffer`/protobuf-es
//! decode step. This relies on serde-native models, i.e. `runtime: Buffa`.

mod bindings;
mod dts;

pub(crate) use bindings::generate_bindings;
pub(crate) use dts::generate_dts;
