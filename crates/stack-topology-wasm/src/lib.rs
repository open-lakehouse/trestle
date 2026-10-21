//! Browser/WASM bindings for the [`olai_stack_topology`] planner.
//!
//! Exposes three functions to JS — [`catalog`](bindings::catalog) (the selectable module catalog),
//! [`plan`](bindings::plan) (plan a selection → graph + services + gateway + rendered
//! artifacts), and [`bundle`](bindings::bundle) (every file of the rendered environment, for a
//! download) — for the `@open-lakehouse/env-editor` component. The planner is pure and
//! `wasm32`-clean (disk I/O is feature-gated out of `olai-stack-topology`), so the whole
//! catalog → plan → render pipeline runs in the browser.
//!
//! The projection itself (planner types → `#[derive(Serialize)]` DTOs, and the pure
//! `plan_result` core) lives in [`olai_stack_topology::projection`], so the native CLI and the
//! browser share one implementation. [`dto`] and [`topology`] re-export it under the paths this
//! crate has always exposed; [`bindings`] holds the thin `#[wasm_bindgen]` JSON-in / JsValue-out
//! wrappers, compiled only on `wasm32` so native builds never need the wasm-bindgen toolchain.

/// The serializable projection (re-exported from [`olai_stack_topology::projection`]).
pub mod dto {
    pub use olai_stack_topology::projection::*;
}

/// The functional topology behind the diagram (re-exported from
/// [`olai_stack_topology::projection::topology`]).
pub mod topology {
    pub use olai_stack_topology::projection::topology::*;
}

#[cfg(target_arch = "wasm32")]
pub mod bindings;
