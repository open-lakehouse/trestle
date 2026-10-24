//! Mount the PyO3 bindings `buffa-e2e`'s build script generated (exported through its
//! `links` metadata) as `crate::codegen`.
//!
//! The generated `mod.rs` starts with an inner `#![allow(..)]`, which `include!` rejects, so
//! instead of including it this writes a one-line shim that mounts it with `#[path]`, making it
//! a real module file exactly as in a consumer crate.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(
        std::env::var("DEP_BUFFA_E2E_PYTHON_DIR")
            .expect("buffa-e2e's build script exports the generated python dir"),
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let shim = format!("#[path = {:?}]\npub mod codegen;\n", dir.join("mod.rs"));
    std::fs::write(out.join("codegen.rs"), shim).unwrap();
}
