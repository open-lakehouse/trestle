// Placeholder until `just regen` runs. `trestle generate` (with `wasm:` output
// configured) overwrites this with `#[wasm_bindgen]` browser bindings, which
// `wasm-pack` then packages into `frontend/src/wasm`. The generated file
// self-gates on `cfg(target_arch = "wasm32")`, so a native build ignores it.
