//! The `#[wasm_bindgen]` surface consumed by `@open-lakehouse/stack-wasm`.
//!
//! Thin wrappers over the pure projection in [`dto`](crate::dto): deserialize JSON in, run the
//! planner, serialize the DTO out. All the interesting logic (and its tests) lives in `dto`; this
//! file is `wasm32`-only glue, so native `cargo test`/`clippy` never pull the wasm toolchain.

use olai_stack_topology::Selection;
use wasm_bindgen::prelude::*;

use crate::dto;

/// Install a panic hook that logs readable Rust panics to the browser console. Called once by the
/// JS wrapper right after the module initializes.
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// The selectable catalog, as a JS object (see `CatalogDto` in the TS types).
///
/// Returns the baseline catalog projected for the picker: every module with its metadata, knobs,
/// requires/conflicts, and provided capability, plus a default selection.
#[wasm_bindgen]
pub fn catalog() -> Result<JsValue, JsValue> {
    let dto = dto::catalog_dto();
    serde_wasm_bindgen::to_value(&dto).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Plan a selection and return the projected result as a JS object (see `PlanResult` in the TS
/// types).
///
/// `selection_json` is a JSON-serialized `Selection` (`{ modules, capabilities, knob_overrides,
/// extra_resources }`). A deserialization failure or a [`PlanError`](olai_stack_topology::PlanError)
/// (unknown/conflicting/cyclic modules, prefix collisions, …) is returned as a rejected `Err`
/// carrying the message string, which the JS wrapper surfaces to the UI.
#[wasm_bindgen]
pub fn plan(selection_json: &str) -> Result<JsValue, JsValue> {
    let selection: Selection = serde_json::from_str(selection_json)
        .map_err(|e| JsValue::from_str(&format!("invalid selection JSON: {e}")))?;
    let result = dto::plan_result(&selection).map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
