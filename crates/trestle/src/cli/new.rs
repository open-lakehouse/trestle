//! `trestle new` — scaffold a new project from a base template (the lakehouse env)
//! plus zero or more apps layered on top.
//!
//! High-level steps:
//! 1. Load the **base** (default: `lakehouse` from the embedded library, or a
//!    custom path/git URL via `--template`/`--base`).
//! 2. Load each **app** named in `--app` (repeatable; default: none).
//! 3. Collect variable values from `--values`, `--set` overrides, prompts, and
//!    manifest defaults. Variable definitions from the base and every app are
//!    merged into a single map.
//! 4. Derive legacy template flags (`with_frontend`, `with_ci`, …) from the
//!    user's category selections so existing `.tmpl` files keep rendering
//!    unchanged during the transition.
//! 5. Resolve the active component set:
//!    - the base's `always:` baseline,
//!    - explicit `--select` picks (and the wizard's category picks once Phase 3
//!      lands),
//!    - each app's `lakehouse_requires:` (hard + soft),
//!    - back-compat: `--profile` + `--with` + `when:` against vars.
//! 6. Aggregate `provides:` across components into a single `stack` context.
//! 7. Render the base's `template/` tree → each app's `template/` tree → each
//!    component's `template/` tree, in topological order.
//! 8. Run every root's `post_init:` hooks.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use clap::Args;
use minijinja::Value;

use crate::embedded::embedded_app_names;
use crate::error::{Error, Result};
use crate::template::hooks::run_post_init;
use crate::template::loader::{LoadedTemplate, load_embedded_app, load_embedded_base};
use crate::template::resolve::{ResolveInput, ScaffoldRoot, resolve_components};
use crate::template::validate::validate_resolved;
use crate::template::wizard::{WizardInput, run_wizard};
use crate::template::{
    ComponentCatalog, Renderer, TemplateSource, aggregate_stack_context, collect_variables,
    load_template, preview, render_tree,
};

#[derive(Args, Clone)]
pub struct NewArgs {
    /// Name of the new project (becomes the destination directory and the
    /// default `project_name` value).
    pub name: String,

    /// Output directory (defaults to `./<name>`).
    #[clap(long, short)]
    pub out_dir: Option<PathBuf>,

    /// Base template (embedded name, git URL, or local path). Defaults to the
    /// embedded `lakehouse` base.
    #[clap(long, short, default_value = "lakehouse")]
    pub template: String,

    /// Apps to layer on top of the base. Repeat for multiple. Each value is an
    /// embedded app name (e.g. `databricks-app-rust`), a git URL, or a local
    /// path.
    #[clap(long = "app", short = 'a', value_name = "APP")]
    pub apps: Vec<String>,

    /// Explicit component / option selections, formatted as
    /// `category=value[,value...]`. App-private categories use the
    /// `app.<app-name>.<category>` namespace. May be repeated.
    #[clap(long = "select", value_name = "CATEGORY=VALUE[,VALUE]", value_parser = parse_selection)]
    pub selections: Vec<(String, Vec<String>)>,

    /// Legacy: named profile (from the base's `profiles:` block). Kept as a
    /// compatibility shim; `--select` is the new way.
    #[clap(long, short)]
    pub profile: Option<String>,

    /// Legacy: extra components to enable by name.
    #[clap(long = "with", short = 'w', value_name = "COMPONENT")]
    pub with: Vec<String>,

    /// YAML file containing variable values, selections, and apps.
    #[clap(long, short)]
    pub values: Option<PathBuf>,

    /// Override individual variables, e.g. `-D project_name=my-app`.
    #[clap(long = "set", short = 'D', value_parser = parse_key_val)]
    pub overrides: Vec<(String, String)>,

    /// Protobuf runtime the generated code should consume: `prost` (default) or
    /// `buffa`. Convenience for `--set runtime=<value>`.
    #[clap(long, value_parser = ["prost", "buffa"])]
    pub runtime: Option<String>,

    /// Skip all prompts; fail if any required variable is unset. Note that
    /// `post_init` hooks marked `confirm: true` are skipped in this mode (they
    /// cannot be confirmed without a prompt).
    #[clap(long)]
    pub non_interactive: bool,

    /// Render into the output directory even if it already exists and is
    /// non-empty. Existing files with the same path as a generated file are
    /// overwritten; other existing files are left untouched.
    #[clap(long)]
    pub force: bool,
}

fn parse_key_val(s: &str) -> std::result::Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got `{s}`"))
}

fn parse_selection(s: &str) -> std::result::Result<(String, Vec<String>), String> {
    let (key, vals) = s
        .split_once('=')
        .ok_or_else(|| format!("expected category=value[,value], got `{s}`"))?;
    let values: Vec<String> = vals
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    Ok((key.to_string(), values))
}

/// Validate the project name before it is used to derive the output directory
/// or the default `project_name` variable.
///
/// The name must match `^[a-z][a-z0-9-]*$` — the same pattern templates declare
/// for the `project_name` variable. Enforcing it here (rather than only later,
/// during variable collection) prevents path-traversal names like `../foo` or
/// absolute paths from being turned into an output directory before the
/// per-variable validation ever runs (notably in `--non-interactive` mode).
fn validate_project_name(name: &str) -> Result<()> {
    let valid = {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            }
            _ => false,
        }
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidVariable {
            name: "project_name".to_string(),
            reason: format!(
                "`{name}` must match `^[a-z][a-z0-9-]*$` (lowercase letters, digits, and dashes; \
                 must start with a letter)"
            ),
        })
    }
}

pub fn run(args: NewArgs) -> Result<()> {
    // Reject invalid / path-traversal names before touching the filesystem.
    validate_project_name(&args.name)?;

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

    // ----- Load base (and optionally each app) -------------------------------
    let base = load_base(&args.template)?;
    let mut apps: Vec<LoadedTemplate> = Vec::new();
    for app in &args.apps {
        apps.push(load_app(app)?);
    }

    // ----- Build initial state from CLI + values file ------------------------
    let mut overrides: BTreeMap<String, String> = args.overrides.iter().cloned().collect();
    if !overrides.contains_key("project_name") {
        overrides.insert("project_name".to_string(), args.name.clone());
    }
    // `--runtime` is a convenience alias for `--set runtime=<value>`; an explicit
    // `--set runtime=...` still wins (insert only if absent).
    if let Some(runtime) = &args.runtime {
        overrides
            .entry("runtime".to_string())
            .or_insert_with(|| runtime.clone());
    }
    let mut selections: BTreeMap<String, Vec<String>> = args.selections.iter().cloned().collect();
    let mut chosen_app_names: Vec<String> = args.apps.clone();

    // Honour the unified --values schema (apps:, selections:, variables:) if a
    // values file was provided.
    if let Some(path) = &args.values {
        merge_structured_values(path, &mut overrides, &mut selections, &mut chosen_app_names)?;
    }

    // ----- Run the wizard (interactive runs only) ----------------------------
    if !args.non_interactive {
        let available_app_templates = load_available_apps()?;
        let available_refs: Vec<&LoadedTemplate> = available_app_templates.iter().collect();
        let catalog = ComponentCatalog::from_embedded()?;
        let wizard_in = WizardInput {
            project_name: &args.name,
            base: &base,
            available_apps: &available_refs,
            preselected_apps: &chosen_app_names,
            preselected_selections: &selections,
            preselected_overrides: &overrides,
            catalog: &catalog,
        };
        let out = run_wizard(wizard_in)?;
        if !out.proceed {
            return Ok(());
        }
        // Merge the wizard's choices back in (wizard never *removes* a CLI
        // pre-fill, it only adds to whatever is missing).
        for (k, v) in out.overrides {
            overrides.entry(k).or_insert(v);
        }
        for (k, v) in out.selections {
            selections.entry(k).or_insert(v);
        }
        for name in out.apps {
            if !chosen_app_names.iter().any(|a| a == &name) {
                chosen_app_names.push(name);
            }
        }

        // Apps the user picked in the wizard need their LoadedTemplate too.
        for name in &chosen_app_names {
            if !apps.iter().any(|a| a.manifest.name == *name) {
                apps.push(load_app(name)?);
            }
        }
    }

    // ----- Merge variables, then collect values ------------------------------
    let merged_manifest = merge_variables(&base, &apps)?;
    let renderer = Renderer::new();
    let mut vars = collect_variables(
        &merged_manifest,
        args.values.as_deref(),
        &overrides,
        args.non_interactive,
    )?;

    // ----- Translate category selections + apps into legacy var flags --------
    apply_app_category_compat(&apps, &selections, &mut vars);

    // ----- Resolve components ------------------------------------------------
    let initial_ctx = build_intermediate_ctx(&vars, &merged_manifest.template_context);

    // The explicit selection list = component-typed picks for lakehouse categories
    // (option strings for app-private categories are *not* components, so they're
    // filtered out by the catalog check).
    let explicit_selections: Vec<String> = collect_component_selections(&base, &apps, &selections);

    let roots: Vec<ScaffoldRoot<'_>> = std::iter::once(ScaffoldRoot {
        root: base.root.as_path(),
        manifest: &base.manifest,
    })
    .chain(apps.iter().map(|a| ScaffoldRoot {
        root: a.root.as_path(),
        manifest: &a.manifest,
    }))
    .collect();

    let resolve_in = ResolveInput {
        roots: &roots,
        vars: &initial_ctx,
        explicit_selections: &explicit_selections,
        profile: args.profile.as_deref(),
        extra_with: &args.with,
    };
    let components = resolve_components(resolve_in, &renderer)?;
    validate_resolved(&components)?;

    let component_manifests: Vec<&_> = components.iter().map(|c| &c.loaded.manifest).collect();
    let stack = aggregate_stack_context(&component_manifests);

    let merged_static_ctx = merge_static_context(&base, &apps);
    let final_ctx = build_final_ctx(&vars, &merged_static_ctx, &stack);

    // ----- Wiring preview ----------------------------------------------------
    let preview_text = preview::render_text(&base, &apps, &components, &stack)?;
    if !args.non_interactive {
        // Show the full preview via cliclack so it sits inside the prompt
        // session visually.
        cliclack::note("Wiring", preview_text.trim_end())
            .map_err(|e| Error::other(format!("preview render failed: {e}")))?;
    } else {
        println!("{preview_text}");
    }

    // ----- Render base, then apps, then components ---------------------------
    let parent_written = render_tree(&base.root, &out_dir, &final_ctx, &renderer)?;
    tracing::info!(
        "rendered {parent_written} files from base `{}`",
        base.manifest.name
    );

    for app in &apps {
        let written = render_tree(&app.root, &out_dir, &final_ctx, &renderer)?;
        tracing::info!("rendered {written} files from app `{}`", app.manifest.name);
    }

    for rc in &components {
        let written = render_tree(&rc.loaded.root, &out_dir, &final_ctx, &renderer)?;
        tracing::info!(
            "rendered {written} files from component `{}`",
            rc.loaded.manifest.name
        );
    }

    // ----- Emit the structured config + derived buf.gen.yaml -----------------
    // `trestle.yaml` / `buf.gen.yaml` are no longer templated; they are produced
    // programmatically from the same selections so they stay in lockstep with the
    // codegen the CLI understands.
    emit_project_config(&args.name, &vars, &out_dir)?;

    // ----- Post-init hooks ---------------------------------------------------
    run_post_init(
        &base.manifest.post_init,
        &out_dir,
        &final_ctx,
        &renderer,
        args.non_interactive,
    )?;
    for app in &apps {
        run_post_init(
            &app.manifest.post_init,
            &out_dir,
            &final_ctx,
            &renderer,
            args.non_interactive,
        )?;
    }

    println!("\nScaffolded `{}` at {}", args.name, out_dir.display());
    if !apps.is_empty() {
        println!("Apps:");
        for app in &apps {
            println!("  - {}", app.manifest.name);
        }
    }
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
    if merged_manifest
        .variables
        .iter()
        .any(|v| v.name == "project_name")
    {
        println!("  just regen     # build proto descriptors + regenerate code");
        println!("  just dev       # run the server (and frontend) in dev mode");
    }
    Ok(())
}

/// Load the base template, defaulting to the embedded `lakehouse` base. Accepts
/// embedded names, git URLs, and local paths via the existing [`TemplateSource`]
/// detection.
fn load_base(spec: &str) -> Result<LoadedTemplate> {
    match TemplateSource::detect(spec) {
        TemplateSource::Embedded(name) => load_embedded_base(&name),
        other => load_template(&other),
    }
}

/// Load an app template by name (embedded), git URL, or local path.
fn load_app(spec: &str) -> Result<LoadedTemplate> {
    match TemplateSource::detect(spec) {
        TemplateSource::Embedded(name) => load_embedded_app(&name),
        other => load_template(&other),
    }
}

/// Load every embedded app template so the wizard can offer them. Errors are
/// degraded to warnings so a corrupt entry in `_apps/` doesn't kill the wizard.
fn load_available_apps() -> Result<Vec<LoadedTemplate>> {
    let mut out = Vec::new();
    for name in embedded_app_names() {
        match load_embedded_app(&name) {
            Ok(t) => out.push(t),
            Err(e) => tracing::warn!("skipping unloadable app `{name}`: {e}"),
        }
    }
    Ok(out)
}

/// Merge a structured `--values` YAML file into the in-memory inputs.
///
/// Accepted top-level keys:
///   apps:        list of app names
///   selections:  map of category → string or list-of-strings
///   variables:   map of variable name → scalar value
///
/// The legacy "flat" form (variable names at top level) is still consumed by
/// `collect_variables`; we only peel off the new structured blocks here.
fn merge_structured_values(
    path: &std::path::Path,
    overrides: &mut BTreeMap<String, String>,
    selections: &mut BTreeMap<String, Vec<String>>,
    apps: &mut Vec<String>,
) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
    let raw: serde_yaml::Value =
        serde_yaml::from_slice(&bytes).map_err(|e| Error::yaml_at(path, e))?;
    let Some(map) = raw.as_mapping() else {
        return Ok(());
    };

    if let Some(serde_yaml::Value::Sequence(seq)) =
        map.get(serde_yaml::Value::String("apps".into()))
    {
        for v in seq {
            if let Some(s) = v.as_str()
                && !apps.iter().any(|a| a == s)
            {
                apps.push(s.to_string());
            }
        }
    }

    if let Some(serde_yaml::Value::Mapping(sel)) =
        map.get(serde_yaml::Value::String("selections".into()))
    {
        for (k, v) in sel {
            let Some(key) = k.as_str() else { continue };
            let values: Vec<String> = match v {
                serde_yaml::Value::String(s) => vec![s.clone()],
                serde_yaml::Value::Sequence(seq) => seq
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
                _ => continue,
            };
            selections.entry(key.to_string()).or_insert(values);
        }
    }

    if let Some(serde_yaml::Value::Mapping(vars)) =
        map.get(serde_yaml::Value::String("variables".into()))
    {
        for (k, v) in vars {
            let Some(key) = k.as_str() else { continue };
            let value = match v {
                serde_yaml::Value::String(s) => s.clone(),
                serde_yaml::Value::Bool(b) => b.to_string(),
                serde_yaml::Value::Number(n) => n.to_string(),
                _ => continue,
            };
            overrides.entry(key.to_string()).or_insert(value);
        }
    }

    Ok(())
}

/// Merge each app's `variables:` into a single combined manifest used for
/// prompt collection. App vars are appended after base vars, deduping by name
/// (first definition wins).
fn merge_variables(
    base: &LoadedTemplate,
    apps: &[LoadedTemplate],
) -> Result<crate::template::Manifest> {
    let mut merged = base.manifest.clone();
    let mut seen: BTreeSet<String> = merged.variables.iter().map(|v| v.name.clone()).collect();
    for app in apps {
        for v in &app.manifest.variables {
            if seen.insert(v.name.clone()) {
                merged.variables.push(v.clone());
            }
        }
    }
    Ok(merged)
}

/// Combine `template_context:` blocks from base + every app into one map. Apps
/// can override base entries (latest app wins). The result is rendered through
/// MiniJinja by `build_final_ctx`.
fn merge_static_context(
    base: &LoadedTemplate,
    apps: &[LoadedTemplate],
) -> BTreeMap<String, serde_yaml::Value> {
    let mut out = base.manifest.template_context.clone();
    for app in apps {
        for (k, v) in &app.manifest.template_context {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// Build the structured [`TrestleConfig`](crate::config::TrestleConfig) from the
/// scaffold's resolved variables and write `trestle.yaml` + the derived
/// `buf.gen.yaml` into the project root. Replaces the old `trestle.yaml.tmpl` /
/// `buf.gen.yaml.tmpl` templating so the scaffolded config always matches what
/// the CLI's `config` / `generate` commands understand.
fn emit_project_config(
    project_name: &str,
    vars: &BTreeMap<String, crate::template::VariableValue>,
    out_dir: &std::path::Path,
) -> Result<()> {
    use crate::config::{
        Bindings, ClientProtocol, Clients, GenerateConfig, Models, NodeClient, ProjectMeta,
        ProtoLib, RustClient, Server, Servers, Transport, TrestleConfig, WasmBindings,
    };

    let bool_var = |key: &str| {
        matches!(
            vars.get(key),
            Some(crate::template::VariableValue::Bool(true))
        )
    };
    let with_frontend = bool_var("with_frontend");
    let with_connect = bool_var("with_connect");
    // Connect RPC and the WASM browser client (frontend) both require buffa, so
    // force it regardless of the `runtime` var when either is on.
    let proto_lib = if with_connect
        || with_frontend
        || vars.get("runtime").and_then(|v| v.as_str()) == Some("buffa")
    {
        ProtoLib::Buffa
    } else {
        ProtoLib::Prost
    };

    // The frontend uses the WASM browser client (no NAPI / TS in the scaffolded
    // app — the browser consumes the wasm bindings directly).
    let node = with_frontend.then(|| NodeClient {
        napi: None,
        ts: None,
        wasm: Some(WasmBindings {
            output: "crates/client/src".to_string(),
        }),
    });

    let mut cfg = TrestleConfig {
        version: crate::config::CONFIG_VERSION,
        project: ProjectMeta {
            name: project_name.to_string(),
            id: None,
            description: None,
        },
        generate: GenerateConfig {
            proto_lib,
            descriptors: "api.bin".to_string(),
            servers: Servers {
                rest: true,
                connect: with_connect,
            },
            clients: Clients {
                rust: Some(RustClient {
                    output: "crates/client/src".to_string(),
                    transport: Transport::Cloud,
                    transport_type_path: None,
                    protocols: vec![ClientProtocol::Rest],
                    connect_client_path: None,
                }),
                python: None,
                node,
            },
            bindings: with_frontend.then(Bindings::default),
            models: Models {
                dir: "crates/common/src/models".to_string(),
                crate_name: None,
                path_template: None,
                path_crate_template: None,
            },
            server: Server {
                output: Some("crates/server/src".to_string()),
                context_type: Some("crate::api::RequestContext".to_string()),
                result_type: Some("crate::api::Result".to_string()),
                ..Server::default()
            },
        },
        enrich_openapi: None,
    };

    cfg.derive_defaults();
    cfg.ensure_id();
    // Connect/WASM both require buffa; force it here rather than prompting mid-scaffold.
    cfg.validate(false)?;

    let trestle_yaml = out_dir.join("trestle.yaml");
    cfg.write(&trestle_yaml, true)?;
    let buf_gen = out_dir.join("buf.gen.yaml");
    fs::write(&buf_gen, crate::config::emit_buf_gen(&cfg.generate))
        .map_err(|e| Error::io_at(&buf_gen, e))?;
    Ok(())
}

/// Project `frontend=react` / `ci=github` selections back onto the legacy
/// `with_frontend` / `with_ci` booleans that the existing `.tmpl` tree still
/// reads. Future work: rewrite the templates to consume the category names
/// directly so this compat layer can go away.
fn apply_app_category_compat(
    apps: &[LoadedTemplate],
    selections: &BTreeMap<String, Vec<String>>,
    vars: &mut BTreeMap<String, crate::template::VariableValue>,
) {
    for app in apps {
        for cat in &app.manifest.categories {
            let key = format!("app.{}.{}", app.manifest.name, cat.id);
            let chosen = selections
                .get(&key)
                .cloned()
                .unwrap_or_else(|| cat.default.clone().into_vec());

            if app.manifest.name == "databricks-app-rust" {
                match cat.id.as_str() {
                    "frontend" => {
                        let on = chosen.iter().any(|v| v == "react");
                        vars.insert(
                            "with_frontend".to_string(),
                            crate::template::VariableValue::Bool(on),
                        );
                    }
                    "ci" => {
                        let on = chosen.iter().any(|v| v == "github");
                        vars.insert(
                            "with_ci".to_string(),
                            crate::template::VariableValue::Bool(on),
                        );
                    }
                    "connect" => {
                        let on = chosen.iter().any(|v| v == "on");
                        vars.insert(
                            "with_connect".to_string(),
                            crate::template::VariableValue::Bool(on),
                        );
                        // ConnectRPC is generated against buffa views, so it
                        // requires the buffa runtime. Force it on when connect is
                        // selected, overriding any `prost` default/pick.
                        if on {
                            vars.insert(
                                "runtime".to_string(),
                                crate::template::VariableValue::String("buffa".to_string()),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Pick out the component-typed selections (i.e. lakehouse categories declared
/// on the base) so the resolver gets them in its explicit-selections channel.
/// App-private category picks (`app.X.frontend=react`) are filtered out — they
/// drive the compat var-fill above and aren't components.
fn collect_component_selections(
    base: &LoadedTemplate,
    apps: &[LoadedTemplate],
    selections: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let lakehouse_category_ids: BTreeSet<&str> = base
        .manifest
        .categories
        .iter()
        .map(|c| c.id.as_str())
        .collect();

    // Gather defaults from base categories that the user didn't explicitly
    // override (everything is "optional", but the defaults survive unless the
    // selection explicitly deselects).
    let mut chosen: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for cat in &base.manifest.categories {
        let picks = match selections.get(&cat.id) {
            Some(v) => v.clone(),
            None => cat.default.clone().into_vec(),
        };
        for name in picks {
            if seen.insert(name.clone()) {
                chosen.push(name);
            }
        }
    }

    // Allow overriding via `--select <unknown-category>=...` to flow through as
    // raw component names. Useful for ad-hoc components that don't belong to
    // any declared category yet.
    for (key, values) in selections {
        if lakehouse_category_ids.contains(key.as_str()) {
            continue;
        }
        if key.starts_with("app.") {
            continue;
        }
        for v in values {
            if seen.insert(v.clone()) {
                chosen.push(v.clone());
            }
        }
    }

    // Drop `none` placeholders that may sneak in from app-private categories;
    // they aren't component names. (App-private picks are filtered above, but
    // belt-and-braces in case someone passes them at the top level.)
    chosen.retain(|n| n != "none");

    let _ = apps;
    chosen
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
