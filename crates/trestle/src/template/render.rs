//! MiniJinja-based renderer with trestle's custom filter set.
//!
//! The renderer wraps a [`minijinja::Environment`] and exposes one-shot helpers for
//! rendering both file paths and file contents. Two flavours of rendering are needed:
//!
//! 1. **Inline rendering** for variable substitution in arbitrary template files.
//!    Templates are added on the fly via `Environment::render_str`.
//! 2. **Expression evaluation** for `when` conditions on components and post-init hooks.

use minijinja::{Environment, Value};

use convert_case::{Case, Casing};

use crate::error::{Error, Result};

/// Renderer wrapping a MiniJinja environment with trestle's filters and globals.
pub struct Renderer {
    pub env: Environment<'static>,
}

impl Renderer {
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);
        register_filters(&mut env);
        register_tests(&mut env);
        Self { env }
    }

    /// Render an inline template against the given context. Used for both file
    /// contents and file paths (filenames may contain `{{var}}` placeholders).
    pub fn render_str(&self, template: &str, ctx: &Value, what: &str) -> Result<String> {
        self.env
            .render_str(template, ctx)
            .map_err(|source| Error::Render {
                file: std::path::PathBuf::from(what),
                source,
            })
    }

    /// Evaluate a boolean expression against the given context. Empty/blank
    /// expressions evaluate to `true` (i.e. "no condition" means "enabled").
    pub fn eval_when(&self, expr: &str, ctx: &Value) -> Result<bool> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Ok(true);
        }
        let compiled = self.env.compile_expression(expr)?;
        let v = compiled.eval(ctx)?;
        Ok(v.is_true())
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Install trestle's filter set on a MiniJinja environment.
pub fn register_filters(env: &mut Environment<'static>) {
    env.add_filter("snake_case", |s: String| s.to_case(Case::Snake));
    env.add_filter("kebab_case", |s: String| s.to_case(Case::Kebab));
    env.add_filter("pascal_case", |s: String| s.to_case(Case::Pascal));
    env.add_filter("camel_case", |s: String| s.to_case(Case::Camel));
    env.add_filter("screaming_snake_case", |s: String| {
        s.to_case(Case::Constant)
    });
    env.add_filter("upper_case", |s: String| s.to_uppercase());
    env.add_filter("lower_case", |s: String| s.to_lowercase());
}

/// Install trestle's test set (`is contains`, etc.).
fn register_tests(_env: &mut Environment<'static>) {
    // Reserved for future expansion (e.g. custom `is shared_component`).
}

/// Wrap a `serde_json::Value`-equivalent map for use as the root MiniJinja context.
///
/// We accept any `serde::Serialize` since `minijinja::Value::from_serialize` makes
/// arbitrary serde types available without manual conversion.
pub fn ctx_from_serialize<S: serde::Serialize>(value: &S) -> Value {
    Value::from_serialize(value)
}
