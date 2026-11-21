//! Node.js code generation module
//!
//! Split into submodules:
//! - `caps`: NAPI capability predicates shared by both emitters
//! - `bindings`: NAPI-RS binding generation (Rust → Node.js wrapper structs)
//! - `typescript`: TypeScript client generation for idiomatic Node.js API

mod bindings;
mod caps;
pub(crate) mod typescript;

pub(crate) use bindings::{generate, main_module};

/// Whether `name` can't name a binding (a parameter or a local) in strict-mode JS or in a `.d.ts`.
fn is_js_reserved(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        // ECMAScript reserved words.
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "new",
        "null",
        "return",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        // Reserved in strict mode (ES modules and classes are strict) and in async functions.
        "await",
        "implements",
        "interface",
        "let",
        "package",
        "private",
        "protected",
        "public",
        "static",
        "yield",
        "arguments",
        "eval",
    ];
    RESERVED.contains(&name)
}
