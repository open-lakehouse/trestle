//! Browser/WASM bindings for the [`olai_stack_topology`] planner.
//!
//! Exposes two functions to JS — [`catalog`](bindings::catalog) (the selectable module catalog)
//! and [`plan`](bindings::plan) (plan a selection → graph + services + gateway + rendered
//! artifacts) — for the `@open-lakehouse/env-editor` component. The planner is pure and
//! `wasm32`-clean (disk I/O is feature-gated out of `olai-stack-topology`), so the whole
//! catalog → plan → render pipeline runs in the browser.
//!
//! The design is a two-layer split:
//!
//! - [`dto`] — the serializable projection (planner types → `#[derive(Serialize)]` DTOs) and the
//!   pure `plan_result` core. Compiled on **all** targets and unit-tested natively, because the
//!   load-bearing work is projecting `Arc<dyn Module>` graph nodes and the non-`Serialize`
//!   gateway config into plain data.
//! - [`bindings`] — the thin `#[wasm_bindgen]` JSON-in / JsValue-out wrappers, compiled only on
//!   `wasm32` so native builds never need the wasm-bindgen toolchain.

pub mod dto;

#[cfg(target_arch = "wasm32")]
pub mod bindings;
