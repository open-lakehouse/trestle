//! `trestle new` — scaffold a new project from a template.
//!
//! The high-level steps are:
//! 1. Resolve the [`TemplateSource`] (embedded / git / local).
//! 2. Materialise the template root on disk and parse its manifest.
//! 3. Collect variable values from `--values`, interactive prompts, or defaults.
//! 4. Resolve the active component set (profile + `--with` + manifest `when:`),
//!    transitively pulling in `depends_on`.
//! 5. Aggregate `provides:` across components into a single `stack` context.
//! 6. Render the parent template's `template/` tree, then each component's, in
//!    topological order (so later layers can override earlier ones).
//! 7. Run `post_init` hooks.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use clap::Args;
use minijinja::Value;

use crate::error::{Error, Result};
use crate::template::hooks::run_post_init;
use crate::template::resolve::{ResolveInput, resolve_components};
use crate::template::{
    Renderer, TemplateSource, aggregate_stack_context, collect_variables, load_template,
    render_tree,
};

#[derive(Args, Clone)]
pub struct NewArgs {
    /// Name of the new project (becomes the destination directory and the
    /// default `project_name`/`lab_name` variable value).
    pub name: String,

    /// Output directory (defaults to `./<name>`).
    #[clap(long, short)]
    pub out_dir: Option<PathBuf>,

    /// Template specifier: an embedded template name, a git URL, or a local path.
    #[clap(long, short, default_value = "databricks-app-rust")]
    pub template: String,

    /// Named profile (from the template manifest's `profiles:` block).
    #[clap(long, short)]
    pub profile: Option<String>,

    /// Additional shared components to enable, on top of the profile.
    #[clap(long = "with", short = 'w', value_name = "COMPONENT")]
    pub with: Vec<String>,

    /// YAML file containing variable values.
    #[clap(long, short)]
    pub values: Option<PathBuf>,

    /// Override individual variables, e.g. `-D project_name=my-app`.
    #[clap(long = "set", short = 'D', value_parser = parse_key_val)]
    pub overrides: Vec<(String, String)>,

    /// Skip all prompts; fail if any required variable is unset.
    #[clap(long)]
    pub non_interactive: bool,

    /// Overwrite the output directory if it already exists.
    #[clap(long)]
    pub force: bool,
}

fn parse_key_val(s: &str) -> std::result::Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got `{s}`"))
}

pub fn run(args: NewArgs) -> Result<()> {
    let out_dir = args
        .out_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(&args.name));
    if out_dir.exists() {
        let is_empty = out_dir
            .read_dir()
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if !is_empty && !args.force {
            return Err(Error::OutputExists(out_dir));
        }
    }
    fs::create_dir_all(&out_dir).map_err(|e| Error::io_at(&out_dir, e))?;

    let source = TemplateSource::detect(&args.template);
    let loaded = load_template(&source)?;
    let template_root = loaded.root.clone();
    let manifest = loaded.manifest.clone();

    // Variables: layer overrides on top of file/interactive/defaults.
    let mut overrides: BTreeMap<String, String> = args.overrides.iter().cloned().collect();
    // Auto-fill the most-common name variable from the positional CLI arg so
    // `trestle new my-app` does the obvious thing.
    for guess in ["project_name", "lab_name"] {
        if manifest.variables.iter().any(|v| v.name == guess) && !overrides.contains_key(guess) {
            overrides.insert(guess.to_string(), args.name.clone());
        }
    }

    let renderer = Renderer::new();

    let vars = collect_variables(
        &manifest,
        args.values.as_deref(),
        &overrides,
        args.non_interactive,
    )?;

    // Initial pass to compose the variable context for `when:` evaluation.
    let initial_ctx = build_intermediate_ctx(&vars, &manifest.template_context);

    let resolve_in = ResolveInput {
        template_root: &template_root,
        manifest: &manifest,
        vars: &initial_ctx,
        profile: args.profile.as_deref(),
        extra_with: &args.with,
    };
    let components = resolve_components(resolve_in, &renderer)?;

    let component_manifests: Vec<&_> = components.iter().map(|c| &c.loaded.manifest).collect();
    let stack = aggregate_stack_context(&component_manifests);

    let final_ctx = build_final_ctx(&vars, &manifest.template_context, &stack);

    // Render parent template tree.
    let parent_written = render_tree(&template_root, &out_dir, &final_ctx, &renderer)?;
    tracing::info!("rendered {parent_written} files from parent template");

    // Render each component on top, in topological order.
    for rc in &components {
        let written = render_tree(&rc.loaded.root, &out_dir, &final_ctx, &renderer)?;
        tracing::info!(
            "rendered {written} files from component `{}`",
            rc.loaded.manifest.name
        );
    }

    // Post-init hooks.
    run_post_init(
        &manifest.post_init,
        &out_dir,
        &final_ctx,
        &renderer,
        args.non_interactive,
    )?;

    println!("\nScaffolded `{}` at {}", args.name, out_dir.display());
    if !components.is_empty() {
        println!("Active components:");
        for rc in &components {
            println!("  - {} ({:?})", rc.loaded.manifest.name, rc.kind);
        }
    }
    println!("\nNext steps:");
    println!("  cd {}", out_dir.display());
    if !stack.components.is_empty() {
        println!("  just up        # bring up the local platform stack");
    }
    if manifest.variables.iter().any(|v| v.name == "project_name") {
        println!("  just regen     # build proto descriptors + regenerate code");
        println!("  just dev       # run the server (and frontend) in dev mode");
    }
    Ok(())
}

fn build_intermediate_ctx(
    vars: &BTreeMap<String, crate::template::VariableValue>,
    static_ctx: &BTreeMap<String, serde_yaml::Value>,
) -> Value {
    let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (k, v) in vars {
        let json = match v {
            crate::template::VariableValue::String(s) => serde_json::Value::String(s.clone()),
            crate::template::VariableValue::Bool(b) => serde_json::Value::Bool(*b),
        };
        map.insert(k.clone(), json);
    }
    for (k, v) in static_ctx {
        if let Ok(j) = serde_json::to_value(v) {
            map.insert(k.clone(), j);
        }
    }
    Value::from_serialize(&map)
}

fn build_final_ctx(
    vars: &BTreeMap<String, crate::template::VariableValue>,
    static_ctx: &BTreeMap<String, serde_yaml::Value>,
    stack: &crate::template::StackContext,
) -> Value {
    let mut map = serde_json::Map::new();
    for (k, v) in vars {
        let json = match v {
            crate::template::VariableValue::String(s) => serde_json::Value::String(s.clone()),
            crate::template::VariableValue::Bool(b) => serde_json::Value::Bool(*b),
        };
        map.insert(k.clone(), json);
    }
    // Static context: rendered through MiniJinja first so authors can use vars there too.
    let renderer = Renderer::new();
    let base = Value::from_serialize(map.clone());
    for (k, v) in static_ctx {
        let rendered = match v {
            serde_yaml::Value::String(s) => renderer
                .render_str(s, &base, &format!("template_context:{k}"))
                .ok()
                .map(serde_json::Value::String)
                .unwrap_or_else(|| serde_json::Value::String(s.clone())),
            other => serde_json::to_value(other).unwrap_or(serde_json::Value::Null),
        };
        map.insert(k.clone(), rendered);
    }
    if let Ok(s) = serde_json::to_value(stack) {
        map.insert("stack".to_string(), s);
    }
    map.insert(
        "trestle".to_string(),
        serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
    );
    Value::from_serialize(serde_json::Value::Object(map))
}
