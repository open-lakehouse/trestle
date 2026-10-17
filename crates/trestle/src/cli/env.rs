//! `trestle env` — create and render composed Docker-Compose lakehouse
//! environments from the `olai-stack-topology` engine.
//!
//! This is the single, user-facing code path for producing composed environments,
//! kept deliberately separate from `trestle new`'s project scaffolding and
//! `trestle generate`'s codegen. Its three subcommands mirror the engine's phases:
//!
//! - [`EnvCommand::New`] — pick modules and how the gateway guards access, optionally
//!   tune knobs, then plan + render into a target directory. Interactive (a
//!   [`cliclack`] wizard) or fully scripted (`--select` / `--auth` / `--policy` /
//!   `--set` / `--prefer`). Every environment gets the gateway (unless
//!   `--no-gateway`); it is not a module to pick.
//! - [`EnvCommand::Render`] — re-render an existing environment from its persisted
//!   `env.toml` ([`EnvManifest`]); ports and routes stay stable by construction.
//! - [`EnvCommand::ListModules`] — enumerate the catalog and the knobs each module
//!   exposes, as text or (`--json`) the catalog projection.
//!
//! Every environment persists an editable `env.toml` alongside its rendered
//! artifacts, so re-rendering and hand-editing are the supported edit loop. Each
//! render also writes [`PLAN_JSON`], the settled plan for tools that join the
//! environment to something else.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use olai_stack_topology::projection::export::{
    ExportMeta, Generator, catalog_export, catalog_export_json, plan_export, plan_export_json,
};
use olai_stack_topology::{
    Catalog, EnvManifest, GatewaySettings, Knob, KnobKind, ManifestError, ModuleId, PlanCtx,
    PlanError, RenderError, Secrets, Selection, TargetId, baseline_catalog, baseline_selection,
    builtin_targets, layout_report,
};

use crate::error::{Error, Result};

/// The `trestle env` subcommand group.
#[derive(Subcommand)]
pub enum EnvCommand {
    /// Create a new composed environment: pick modules, set knobs, render.
    New(Box<EnvNewArgs>),
    /// Re-render an existing environment from its `env.toml`.
    Render(EnvRenderArgs),
    /// List catalog modules and the knobs each exposes.
    ListModules(EnvListModulesArgs),
}

#[derive(Args, Clone)]
pub struct EnvNewArgs {
    /// Environment / compose project name. Also the default output directory.
    pub name: String,

    /// Output directory (defaults to `./<name>`).
    #[clap(long, short)]
    pub out_dir: Option<PathBuf>,

    /// Module ids to select, e.g. `--select postgres,mlflow`. Repeatable and
    /// comma-separated. The gateway is included unless `--no-gateway`. When empty in
    /// interactive mode the wizard starts from the default lakehouse selection.
    #[clap(long = "select", value_name = "MODULE[,MODULE]", value_delimiter = ',')]
    pub select: Vec<String>,

    /// Require sign-in on the gateway's API and UI routes (bundled Authelia).
    #[clap(long)]
    pub auth: bool,

    /// Authorize every signed-in request with a Rego policy (bundled OPA).
    /// Implies `--auth`.
    #[clap(long)]
    pub policy: bool,

    /// Run without a gateway: publish each service on its own host port (e.g. Unity
    /// Catalog at `localhost:8080`). For bare deployments of a service and its backends.
    #[clap(long, conflicts_with_all = ["auth", "policy"])]
    pub no_gateway: bool,

    /// Knob override, `module.key=value` (e.g. `--set postgres.image=postgres:17`).
    /// Repeatable. A value set here is not asked for again by the wizard.
    #[clap(long = "set", value_name = "MODULE.KEY=VALUE", value_parser = parse_knob_override)]
    pub knobs: Vec<((String, String), String)>,

    /// Prefer a provider for a resource role, `role=provider[,provider...]`
    /// (e.g. `--prefer object_store=azurite,rustfs`). Repeatable.
    #[clap(long = "prefer", value_name = "ROLE=PROVIDER[,PROVIDER]", value_parser = parse_selection)]
    pub prefer: Vec<(String, Vec<String>)>,

    /// Deploy target the environment runs on (default `compose`, Docker Compose).
    #[clap(long, value_name = "TARGET")]
    pub target: Option<String>,

    /// Gateway host-published port (default 9080).
    #[clap(long)]
    pub gateway_host_port: Option<u16>,

    /// Root data directory injected as `${DATA_ROOT}` (default `./.data`).
    #[clap(long)]
    pub data_root: Option<String>,

    /// Generate a Python client workspace (uv + marimo) under `client/`.
    #[clap(long)]
    pub client: bool,

    /// Also add the selected services' example notebooks to the client
    /// workspace. Implies `--client`.
    #[clap(long)]
    pub client_examples: bool,

    /// Skip all prompts and take the recommended defaults: the default lakehouse
    /// selection (unless `--select` is given) with every knob at its default
    /// (unless `--set`). Customize later by editing `env.toml`.
    #[clap(long, short = 'y', conflicts_with = "non_interactive")]
    pub defaults: bool,

    /// Ask every knob in the wizard, including advanced ones (container images,
    /// log levels), instead of offering the recommended defaults first.
    #[clap(long, conflicts_with_all = ["defaults", "non_interactive"])]
    pub advanced: bool,

    /// Skip all prompts. A required knob with no value is an error in this mode.
    #[clap(long)]
    pub non_interactive: bool,

    /// Render into the output directory even if it already exists and is
    /// non-empty. Generated files are overwritten; other files are left alone.
    #[clap(long)]
    pub force: bool,
}

#[derive(Args, Clone)]
pub struct EnvRenderArgs {
    /// Directory containing `env.toml` (defaults to the current directory).
    #[clap(default_value = ".")]
    pub dir: PathBuf,

    /// Write rendered artifacts here (defaults to the `env.toml` directory).
    #[clap(long, short)]
    pub out_dir: Option<PathBuf>,

    /// Render even if the output directory is non-empty (overwrites generated files).
    #[clap(long)]
    pub force: bool,
}

#[derive(Args, Clone)]
pub struct EnvListModulesArgs {
    /// Also print each module's knobs.
    #[clap(long)]
    pub knobs: bool,

    /// Print the catalog as JSON: every module with its model ref, role, knobs, and helpers,
    /// plus the catalog's identity (as `plan.json` records it).
    #[clap(long)]
    pub json: bool,

    /// Include the module manifests the `env.toml` in this directory lists.
    #[clap(long)]
    pub dir: Option<PathBuf>,
}

/// Parse a `module.key=value` override. The value is split on the *first* `=`
/// (so a value may itself contain `=`, e.g. a connection string), and the
/// `module.key` left-hand side on its *first* `.`.
fn parse_knob_override(s: &str) -> std::result::Result<((String, String), String), String> {
    let (lhs, value) = s
        .split_once('=')
        .ok_or_else(|| format!("expected module.key=value, got `{s}`"))?;
    let (module, knob) = lhs
        .split_once('.')
        .ok_or_else(|| format!("expected module.key=value, got `{s}`"))?;
    let (module, knob) = (module.trim(), knob.trim());
    if module.is_empty() || knob.is_empty() {
        return Err(format!("expected module.key=value, got `{s}`"));
    }
    Ok(((module.to_string(), knob.to_string()), value.to_string()))
}

/// Parse a `key=value[,value...]` selection (shared shape with `trestle new`'s
/// `--select`). Used here for `--prefer role=provider[,provider]`.
fn parse_selection(s: &str) -> std::result::Result<(String, Vec<String>), String> {
    let (key, vals) = s
        .split_once('=')
        .ok_or_else(|| format!("expected key=value[,value], got `{s}`"))?;
    let values: Vec<String> = vals
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect();
    Ok((key.to_string(), values))
}

pub fn run(cmd: EnvCommand) -> Result<()> {
    match cmd {
        EnvCommand::New(args) => run_new(*args),
        EnvCommand::Render(args) => run_render(args),
        EnvCommand::ListModules(args) => run_list_modules(args),
    }
}

// ---------------------------------------------------------------------------
// env new
// ---------------------------------------------------------------------------

fn run_new(args: EnvNewArgs) -> Result<()> {
    validate_env_name(&args.name)?;
    let out_dir = args
        .out_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(&args.name));
    ensure_writable_dir(&out_dir, args.force)?;

    let catalog = baseline_catalog();

    // `--defaults` and `--non-interactive` both skip every prompt; only the
    // latter is strict about an explicit selection.
    let interactive = !args.non_interactive && !args.defaults;

    // Seed the selection + knob overrides from CLI flags. When nothing was
    // selected, start from the default lakehouse: as the wizard's pre-checks, or
    // as-is under `--defaults`.
    let mut selection = if args.select.is_empty() && !args.non_interactive {
        baseline_selection()
    } else {
        Selection::modules(args.select.iter().cloned())
    };
    selection.gateway = if args.no_gateway {
        GatewaySettings::DIRECT
    } else {
        GatewaySettings {
            auth: args.auth,
            policy: args.policy,
            ..GatewaySettings::OPEN
        }
    };
    let access_preset = args.auth || args.policy || args.no_gateway;
    for ((module, knob), value) in &args.knobs {
        selection
            .knob_overrides
            .entry(ModuleId::from(module.as_str()))
            .or_default()
            .insert(knob.clone(), value.clone());
    }
    selection.client.python = args.client || args.client_examples;
    selection.client.examples = args.client_examples;

    // The context carries the environment-level facts the planner can't derive.
    let mut ctx = PlanCtx {
        env_name: args.name.clone(),
        ..PlanCtx::default()
    };
    if let Some(target) = &args.target {
        ctx.target = TargetId::from(target.as_str());
    }
    if let Some(port) = args.gateway_host_port {
        ctx.gateway_host_port = port;
    }
    if let Some(root) = &args.data_root {
        ctx.data_root = root.clone();
    }
    for (role, providers) in &args.prefer {
        ctx.provider_preference.insert(
            role.clone(),
            providers
                .iter()
                .map(|p| ModuleId::from(p.as_str()))
                .collect(),
        );
    }

    // Interactive: let the wizard pick modules and fill the knobs the user wants
    // to change (by default none). The client workspace is only asked about when
    // no `--client*` flag already decided it.
    if interactive {
        let mode = args.advanced.then_some(wizard::Mode::All);
        let ask_client = !(args.client || args.client_examples);
        wizard::run(
            &catalog,
            &ctx,
            &mut selection,
            access_preset,
            mode,
            ask_client,
        )?;
    }

    // Plan, then materialize + persist. This is the shared core with `env render`.
    // Secrets are runtime-only (never persisted): resolve them for this render only.
    let plan = match catalog.plan(
        &selection,
        &PlanCtx {
            secrets: Some(load_secrets(&out_dir)?),
            ..ctx.clone()
        },
    ) {
        // A hosted provider needs secrets that aren't set yet: keep the choices in env.toml so
        // setting them and running `env render` picks up where this left off.
        Err(PlanError::UnresolvedSecrets { vars }) => {
            let preview = catalog.plan(&selection, &ctx).map_err(map_plan_err)?;
            write_manifest(&preview, &selection, &ctx, &out_dir)?;
            return Err(Error::other(format!(
                "hosted resources reference unset secrets: {}",
                vars.join(", ")
            ))
            .with_hint(format!(
                "saved {}/env.toml; export them (or set them in `{SECRETS_FILE}` beside it), \
                 then run `trestle env render {}`",
                out_dir.display(),
                out_dir.display()
            )));
        }
        plan => plan.map_err(map_plan_err)?,
    };

    let summary = config_summary(&plan, &selection, &out_dir);
    if !interactive {
        println!("{summary}\n");
        println!("{}", layout_report(&plan));
    } else {
        cliclack::note("Configuration", summary).map_err(io_err)?;
        cliclack::note("Gateway layout", layout_report(&plan)).map_err(io_err)?;
        let proceed = cliclack::confirm("Render this environment?")
            .initial_value(true)
            .interact()
            .map_err(io_err)?;
        if !proceed {
            cliclack::outro_cancel("Cancelled.").map_err(io_err)?;
            return Ok(());
        }
    }

    write_env(&plan, &selection, &ctx, &out_dir)?;
    report_written(&plan, &out_dir);
    Ok(())
}

/// Materialize a plan to `dir` and persist the `env.toml` manifest beside it.
/// The pure core shared by `env new` and `env render`, with no prompting so it
/// is directly exercisable in tests.
fn write_env(
    plan: &olai_stack_topology::Plan,
    selection: &Selection,
    ctx: &PlanCtx,
    dir: &std::path::Path,
) -> Result<()> {
    let manifest = EnvManifest::new(selection.clone(), ctx.clone());
    write_rendered(plan, &manifest, &baseline_catalog(), dir)?;
    write_manifest(plan, selection, ctx, dir)
}

/// The file every render writes beside `compose.yaml`: the settled plan as JSON.
pub const PLAN_JSON: &str = "plan.json";

/// Materialize `plan` (planned from `manifest` against `catalog`) to `dir`, with its
/// [`PLAN_JSON`] export of the same plan beside the files.
fn write_rendered(
    plan: &olai_stack_topology::Plan,
    manifest: &EnvManifest,
    catalog: &Catalog,
    dir: &Path,
) -> Result<()> {
    let materialized = plan.materialize().map_err(map_render_err)?;
    let export = plan_export(plan, &materialized, manifest, catalog, &export_meta())
        .and_then(|export| plan_export_json(&export))
        .map_err(|e| Error::other(format!("serializing {PLAN_JSON}: {e}")))?;
    materialized
        .write_to(dir)
        .map_err(|e| Error::io_at(dir, e))?;
    let path = dir.join(PLAN_JSON);
    std::fs::write(&path, export).map_err(|e| Error::io_at(&path, e))
}

/// This build's provenance for [`PLAN_JSON`]. `TRESTLE_REVISION`, when set at build time,
/// records the source revision; without it the output stays identical across rebuilds.
fn export_meta() -> ExportMeta {
    ExportMeta::new(Generator {
        name: "trestle".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        revision: option_env!("TRESTLE_REVISION").map(str::to_string),
    })
}

/// The catalog `manifest` plans against: the baseline, with the module manifests its
/// `[catalog] manifests` lists (relative to `dir`) merged over it in order, so a manifest
/// module with a baseline id replaces that module.
fn load_catalog(manifest: &EnvManifest, dir: &Path) -> Result<Catalog> {
    let mut catalog = baseline_catalog();
    for rel in &manifest.catalog.manifests {
        let path = dir.join(rel);
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io_at(&path, e))?;
        let custom =
            Catalog::from_manifests([text.as_str()]).map_err(|e| Error::yaml_at(&path, e))?;
        catalog = catalog.merge(custom);
    }
    Ok(catalog)
}

/// Persist the `env.toml` manifest for `selection` in `dir`, followed by a commented reference
/// of what `plan` leaves at its defaults (knobs, hosted connections).
fn write_manifest(
    plan: &olai_stack_topology::Plan,
    selection: &Selection,
    ctx: &PlanCtx,
    dir: &std::path::Path,
) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io_at(dir, e))?;
    let manifest = EnvManifest::new(selection.clone(), ctx.clone());
    let manifest_path = dir.join("env.toml");
    manifest
        .write_to(&manifest_path)
        .map_err(map_manifest_err)?;
    // Defaults are listed as comments, not written as overrides, so a later
    // trestle release's defaults (e.g. image bumps) still apply on re-render.
    let reference = knob_reference(plan, selection) + &hosted_reference(plan, selection);
    if !reference.is_empty() {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&manifest_path)
            .and_then(|mut f| f.write_all(reference.as_bytes()))
            .map_err(|e| Error::io_at(&manifest_path, e))?;
    }
    Ok(())
}

/// The knob values `selection` overrides, as `module.key = value` lines.
fn non_default_knobs(selection: &Selection) -> Vec<String> {
    selection
        .knob_overrides
        .iter()
        .flat_map(|(module, knobs)| {
            knobs
                .iter()
                .map(move |(key, value)| format!("{module}.{key} = {value}"))
        })
        .collect()
}

/// A short, human summary of what is about to be rendered: the planned modules
/// and only the knobs that differ from their defaults.
fn config_summary(
    plan: &olai_stack_topology::Plan,
    selection: &Selection,
    dir: &std::path::Path,
) -> String {
    let modules: Vec<String> = plan
        .graph
        .nodes
        .iter()
        .map(|m| m.id().to_string())
        .collect();
    let access = match (&plan.gateway.auth, &plan.gateway.policy) {
        (None, _) => "open (no sign-in)".to_string(),
        (Some(auth), None) => format!("sign-in ({})", auth.cluster),
        (Some(auth), Some(policy)) => {
            format!("sign-in + policy ({}, {})", auth.cluster, policy.cluster)
        }
    };
    let overrides = non_default_knobs(selection);
    let settings = if overrides.is_empty() {
        "all defaults".to_string()
    } else {
        format!("\n  {}", overrides.join("\n  "))
    };
    format!(
        "Modules:  {}\nGateway:  {access}\nSettings: {settings}\nCustomize later: edit {}/env.toml, then `trestle env render {}`",
        modules.join(", "),
        dir.display(),
        dir.display(),
    )
}

/// A commented TOML block for the end of `env.toml` that lists every knob of the
/// planned modules that `selection` leaves at its default, with that default. It
/// makes the edit surface discoverable without pinning anything.
///
/// A module with no overrides gets a commented table header to uncomment; one
/// that already has a `[selection.knob_overrides.<id>]` table above only lists
/// its remaining keys, since TOML forbids defining the same table twice.
fn knob_reference(plan: &olai_stack_topology::Plan, selection: &Selection) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    for module in &plan.graph.nodes {
        let id = module.id();
        let overridden = selection.knob_overrides.get(id);
        let remaining: Vec<&Knob> = module
            .knobs()
            .iter()
            .filter(|k| overridden.is_none_or(|o| !o.contains_key(&k.key)))
            .filter(|k| k.default.is_some())
            .collect();
        if remaining.is_empty() {
            continue;
        }
        out.push('\n');
        if overridden.is_some() {
            let _ = writeln!(out, "# more for [selection.knob_overrides.{id}] above:");
        } else {
            let _ = writeln!(out, "# [selection.knob_overrides.{id}]");
        }
        for knob in remaining {
            let default = knob.default.as_deref().unwrap_or_default();
            let _ = writeln!(out, "# {} = {}", knob.key, toml_string(default));
        }
    }
    if out.is_empty() {
        return out;
    }
    format!(
        "\n# --- Available knobs (defaults shown; uncomment to override) ---\n\
         # Values are strings. See `trestle env list-modules --knobs` for what each does.\n{out}"
    )
}

/// A commented TOML block for the end of `env.toml` showing the connection each planned hosted
/// provider uses by default (`selection` doesn't configure it), to uncomment and edit, plus the
/// env vars its `${VAR}` references read.
fn hosted_reference(plan: &olai_stack_topology::Plan, selection: &Selection) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    for (id, resource) in &plan.hosted {
        if selection.hosted.contains_key(id) {
            continue;
        }
        let vars: Vec<String> = resource.connection.secret_refs().into_iter().collect();
        // Nested maps over the resource itself (not a `toml::Table`, which sorts keys) keep the
        // connection's fields in declaration order, `kind` first.
        let table = std::collections::BTreeMap::from([(
            "selection",
            std::collections::BTreeMap::from([(
                "hosted",
                std::collections::BTreeMap::from([(id, resource)]),
            )]),
        )]);
        let block = toml::to_string_pretty(&table).expect("a hosted resource serializes");
        out.push('\n');
        if !vars.is_empty() {
            let _ = writeln!(out, "# {id} reads: {}", vars.join(", "));
        }
        for line in block.lines().filter(|l| !l.is_empty()) {
            let _ = writeln!(out, "# {line}");
        }
    }
    if out.is_empty() {
        return out;
    }
    format!(
        "\n# --- Hosted connections (defaults shown; uncomment to configure) ---\n\
         # Set the `${{VAR}}` secrets in the environment or `{SECRETS_FILE}`; never write them here.\n{out}"
    )
}

/// `value` as a TOML basic string literal.
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn report_written(plan: &olai_stack_topology::Plan, dir: &std::path::Path) {
    print!("{}", next_steps(plan, dir));
}

/// What to do after rendering: start the stack, where it is reachable, and the generated
/// files a user is expected to read or edit.
fn next_steps(plan: &olai_stack_topology::Plan, dir: &std::path::Path) -> String {
    use std::fmt::Write as _;

    let dir = dir.display();
    let mut out = format!("Wrote composed environment to {dir}\n\nNext steps:\n");
    let _ = writeln!(out, "  cd {dir} && docker compose up -d --wait");
    match plan.gateway_url() {
        Some(url) => {
            let _ = writeln!(out, "  Gateway:   {url}  (routes: LAYOUT.md)");
        }
        None => {
            let _ = writeln!(
                out,
                "  Services:  no gateway; each on its own host port  (URLs: LAYOUT.md)"
            );
        }
    }
    if let Some(secure) = &plan.gateway.secure {
        let _ = writeln!(
            out,
            "  Trust:     docker compose cp {}:{} .  (the gateway's local CA, for your browser or `curl --cacert`)",
            secure.mint_service, secure.trust.ca_pem
        );
    }
    if plan.gateway.auth.is_some() {
        let _ = writeln!(
            out,
            "  Sign in:   demo / authelia by default  (accounts: users.yml; APIs take basic auth)"
        );
    }
    if plan.gateway.policy.is_some() {
        let _ = writeln!(
            out,
            "  Policy:    policies/authz.rego  (yours to edit; check with `opa test policies/`)"
        );
    }
    let _ = writeln!(
        out,
        "  Clients:   {} (host) or {} (in-network): endpoints only, no credentials",
        olai_stack_topology::CLIENT_HOST_ENV_PATH,
        olai_stack_topology::CLIENT_CONTAINER_ENV_PATH,
    );
    if let Some(url) = plan.client_env_host.get("MARIMO_URL") {
        // The stack serves the workspace itself (the `marimo` module).
        let _ = writeln!(
            out,
            "  Notebooks: {url}/  (hosted; edits land in {dir}/{})",
            olai_stack_topology::PYTHON_CLIENT_DIR,
        );
    } else if plan.python_client.is_some() {
        let _ = writeln!(
            out,
            "  Notebooks: cd {dir}/{} && uv sync && uv run marimo edit notebooks",
            olai_stack_topology::PYTHON_CLIENT_DIR,
        );
    }
    if let Some(url) = plan.client_env_host.get("PUPPYGRAPH_URL") {
        let _ = writeln!(
            out,
            "  Graph:     {url}  (schema: graphs/puppygraph/schema.json; then `docker compose up puppygraph-schema`)"
        );
    }
    if let Some(url) = plan.client_env_host.get("RIVERS_URL") {
        let _ = writeln!(
            out,
            "  Pipelines: {url}  (code: pipelines/rivers/; reload the code location from the UI after editing)"
        );
    }
    let _ = writeln!(
        out,
        "  Re-render: edit env.toml, then `trestle env render {dir}`"
    );
    out
}

// ---------------------------------------------------------------------------
// env render
// ---------------------------------------------------------------------------

fn run_render(args: EnvRenderArgs) -> Result<()> {
    let manifest_path = args.dir.join("env.toml");
    let mut manifest = EnvManifest::read_from(&manifest_path).map_err(|e| {
        map_manifest_err(e).with_hint(format!(
            "expected an `env.toml` in {} — create one with `trestle env new`",
            args.dir.display()
        ))
    })?;

    let catalog = load_catalog(&manifest, &args.dir)?;
    manifest.context.secrets = Some(load_secrets(&args.dir)?);
    let plan = manifest.plan(&catalog).map_err(map_plan_err)?;

    let out_dir = args.out_dir.clone().unwrap_or_else(|| args.dir.clone());
    // The manifest dir is expected to be non-empty (it holds env.toml); only
    // guard a *different* output dir against clobbering unrelated files.
    if out_dir != args.dir {
        ensure_writable_dir(&out_dir, args.force)?;
    }

    println!("{}", layout_report(&plan));
    write_rendered(&plan, &manifest, &catalog, &out_dir)?;
    report_written(&plan, &out_dir);
    Ok(())
}

// ---------------------------------------------------------------------------
// env list-modules
// ---------------------------------------------------------------------------

fn run_list_modules(args: EnvListModulesArgs) -> Result<()> {
    let (catalog, manifests) = match &args.dir {
        Some(dir) => {
            let manifest =
                EnvManifest::read_from(&dir.join("env.toml")).map_err(map_manifest_err)?;
            (load_catalog(&manifest, dir)?, manifest.catalog.manifests)
        }
        None => (baseline_catalog(), Vec::new()),
    };
    if args.json {
        let listing = catalog_export(&catalog, &manifests, &export_meta())
            .and_then(|export| catalog_export_json(&export))
            .map_err(|e| Error::other(format!("serializing the catalog: {e}")))?;
        print!("{listing}");
        return Ok(());
    }
    for module in catalog.modules() {
        let id = module.id();
        let name = module.display_name().unwrap_or(id.as_str());
        let category = module
            .category()
            .map(|c| format!("  [{c}]"))
            .unwrap_or_default();
        let notebooks = if module.provides().python.notebooks.is_empty() {
            ""
        } else {
            "  (+notebooks)"
        };
        // The planner adds these itself; say how instead of implying `--select`.
        let category = match module.provider_of() {
            _ if module.hosted().is_some() => format!("{category} (hosted: not run)"),
            Some(olai_stack_topology::Role::GATEWAY) => format!("{category} (always included)"),
            Some(olai_stack_topology::Role::AUTH) => format!("{category} (enabled by --auth)"),
            Some(olai_stack_topology::Role::POLICY) => format!("{category} (enabled by --policy)"),
            _ => category,
        };
        match module.summary() {
            Some(summary) => println!("{id}  {name} — {summary}{category}{notebooks}"),
            None => println!("{id}  {name}{category}{notebooks}"),
        }
        if args.knobs {
            for knob in module.knobs() {
                println!("    {}", fmt_knob(knob));
            }
        }
    }
    Ok(())
}

/// One line describing a knob for `env list-modules --knobs`.
fn fmt_knob(knob: &Knob) -> String {
    let kind = match &knob.kind {
        KnobKind::String => "string".to_string(),
        KnobKind::Bool => "bool".to_string(),
        KnobKind::Enum { options } => format!("enum{{{}}}", options.join("|")),
        KnobKind::Integer { min, max } => match (min, max) {
            (Some(lo), Some(hi)) => format!("integer[{lo}..={hi}]"),
            (Some(lo), None) => format!("integer[{lo}..]"),
            (None, Some(hi)) => format!("integer[..={hi}]"),
            (None, None) => "integer".to_string(),
        },
        KnobKind::Port => "port".to_string(),
    };
    let mut line = format!("{} ({kind})", knob.key);
    if knob.required {
        line.push_str(" [required]");
    }
    if knob.advanced {
        line.push_str(" [advanced]");
    }
    if let Some(default) = &knob.default {
        line.push_str(&format!(" default={default}"));
    }
    if let Some(help) = &knob.help {
        line.push_str(&format!(" — {help}"));
    }
    line
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Validate the environment name before it derives an output directory. Matches
/// `^[a-z][a-z0-9-]*$` (as `trestle new` does for project names) so a name can't
/// escape into a path-traversal target.
fn validate_env_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidVariable {
            name: "env name".to_string(),
            reason: format!(
                "`{name}` must match `^[a-z][a-z0-9-]*$` (lowercase letters, digits, and dashes; \
                 must start with a letter)"
            ),
        })
    }
}

/// Create `dir` (and parents), erroring if it already exists non-empty and
/// `force` is not set — mirroring `trestle new`.
fn ensure_writable_dir(dir: &std::path::Path, force: bool) -> Result<()> {
    if dir.exists() {
        let non_empty = dir
            .read_dir()
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if non_empty && !force {
            return Err(Error::OutputExists(dir.to_path_buf()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io_at(dir, e))?;
    Ok(())
}

/// The file beside `env.toml` that may supply secrets for hosted resources (git-ignored).
const SECRETS_FILE: &str = "secrets.env";

/// The secrets `${VAR}` references in hosted resources resolve against: `KEY=VALUE` lines
/// from [`SECRETS_FILE`] in `dir` (if present), overridden by the process environment.
fn load_secrets(dir: &std::path::Path) -> Result<Secrets> {
    let mut secrets = Secrets::new();
    let path = dir.join(SECRETS_FILE);
    if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io_at(&path, e))?;
        for (name, value) in parse_env_lines(&text) {
            secrets.insert(name, value);
        }
    }
    for (name, value) in std::env::vars() {
        secrets.insert(name, value);
    }
    Ok(secrets)
}

/// Parse dotenv-style `KEY=VALUE` lines: blank lines and `#` comments are skipped, an
/// `export ` prefix is allowed, and one pair of matching surrounding quotes is stripped.
fn parse_env_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let l = l.strip_prefix("export ").unwrap_or(l);
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = ['"', '\'']
                .iter()
                .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

fn map_plan_err(e: PlanError) -> Error {
    let hint = match &e {
        PlanError::UnresolvedSecrets { .. } => format!(
            "export them, or set them in `{SECRETS_FILE}` beside env.toml (it is git-ignored)"
        ),
        PlanError::UnknownTarget(_) => {
            let targets: Vec<String> = builtin_targets().iter().map(|t| t.id().0).collect();
            format!("valid deploy targets: {}", targets.join(", "))
        }
        _ => "run `trestle env list-modules` to see valid module ids and knobs".to_string(),
    };
    Error::other(format!("planning the environment failed: {e}")).with_hint(hint)
}

fn map_render_err(e: RenderError) -> Error {
    Error::other(format!("rendering the environment failed: {e}"))
}

fn map_manifest_err(e: ManifestError) -> Error {
    Error::other(format!("environment manifest error: {e}"))
}

fn io_err(e: std::io::Error) -> Error {
    Error::PlainIo(e)
}

// ---------------------------------------------------------------------------
// interactive wizard
// ---------------------------------------------------------------------------

mod wizard {
    use super::{
        Catalog, GatewaySettings, Knob, KnobKind, ModuleId, PlanCtx, Result, Selection, io_err,
        map_plan_err,
    };
    use olai_stack_topology::{Module, Role};
    use std::collections::BTreeMap;

    /// How much of the knob surface the wizard asks about.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        /// Ask nothing: every knob the CLI didn't `--set` takes its default.
        Defaults,
        /// Ask only the knobs that shape the environment (not [`Knob::advanced`]).
        Key,
        /// Ask the key knobs, then offer each module's advanced knobs.
        All,
    }

    /// Drive the interactive create flow, mutating `selection` in place: pick the
    /// module set and how the gateway guards access (unless `access_preset`), choose
    /// how much to configure (unless `mode` is preset), fill the knobs that mode
    /// covers for every module the plan will run (the gateway included), then (when
    /// `ask_client`) whether to generate the Python client workspace.
    pub fn run(
        catalog: &Catalog,
        ctx: &PlanCtx,
        selection: &mut Selection,
        access_preset: bool,
        mode: Option<Mode>,
        ask_client: bool,
    ) -> Result<()> {
        cliclack::intro("trestle env new").map_err(io_err)?;

        selection.modules = ask_modules(catalog, &selection.modules)?;
        if !access_preset {
            selection.gateway = ask_access(selection.gateway)?;
        }
        let mode = match mode {
            Some(mode) => mode,
            None => ask_mode()?,
        };
        if mode != Mode::Defaults {
            let planned: Vec<ModuleId> = catalog
                .plan(selection, ctx)
                .map_err(map_plan_err)?
                .graph
                .nodes
                .iter()
                .map(|m| m.id().clone())
                .collect();
            configure_knobs(catalog, &planned, &mut selection.knob_overrides, mode)?;
        }
        if ask_client {
            ask_client_workspace(catalog, selection)?;
        }

        Ok(())
    }

    /// Ask whether to generate the Python client workspace and, when any selected module
    /// ships example notebooks, whether to include them.
    fn ask_client_workspace(catalog: &Catalog, selection: &mut Selection) -> Result<()> {
        selection.client.python =
            cliclack::confirm("Generate a Python client workspace (uv + marimo)?")
                .initial_value(false)
                .interact()
                .map_err(io_err)?;
        if !selection.client.python {
            return Ok(());
        }
        // Only directly-selected modules are listed; dependencies pulled in by the planner
        // contribute their notebooks too.
        let with_examples: Vec<&str> = selection
            .modules
            .iter()
            .filter_map(|id| catalog.get(id))
            .filter(|m| !m.provides().python.notebooks.is_empty())
            .map(|m| m.id().as_str())
            .collect();
        if !with_examples.is_empty() {
            selection.client.examples = cliclack::confirm(format!(
                "Include example notebooks ({})?",
                with_examples.join(", ")
            ))
            .initial_value(true)
            .interact()
            .map_err(io_err)?;
        }
        Ok(())
    }

    /// Whether `module` is one the planner adds on its own (see [`Role::GATEWAY_MANAGED`]).
    fn gateway_managed(module: &dyn Module) -> bool {
        module
            .provider_of()
            .is_some_and(|role| Role::GATEWAY_MANAGED.contains(&role))
    }

    /// Multi-select the module set, pre-checking `preselected`. The gateway and its
    /// sign-in / policy providers are not offered: the plan adds them itself. A
    /// preselected one (from `--select`) is kept as-is.
    fn ask_modules(catalog: &Catalog, preselected: &[ModuleId]) -> Result<Vec<ModuleId>> {
        let items: Vec<(String, String, String)> = catalog
            .modules()
            .iter()
            .filter(|m| !gateway_managed(m.as_ref()))
            .map(|m| {
                let id = m.id().to_string();
                let label = m.display_name().unwrap_or(&id).to_string();
                let hint = m.summary().unwrap_or("").to_string();
                (id, label, hint)
            })
            .collect();
        let initial: Vec<String> = preselected.iter().map(ToString::to_string).collect();

        let picked: Vec<String> =
            cliclack::multiselect("Select modules (space to toggle, enter to confirm)")
                .items(&items)
                .initial_values(initial)
                .required(true)
                .interact()
                .map_err(io_err)?;
        let managed = preselected
            .iter()
            .filter(|id| catalog.get(id).is_some_and(|m| gateway_managed(m.as_ref())));
        Ok(managed
            .cloned()
            .chain(picked.into_iter().map(ModuleId::from))
            .collect())
    }

    /// How the gateway guards access: open, sign-in, or sign-in plus policy — or no
    /// gateway at all.
    fn ask_access(current: GatewaySettings) -> Result<GatewaySettings> {
        cliclack::select("Gateway access?")
            .item(GatewaySettings::OPEN, "Open", "no sign-in")
            .item(
                GatewaySettings::SIGN_IN,
                "Sign-in",
                "single-sign-on for APIs and UIs (Authelia)",
            )
            .item(
                GatewaySettings::POLICY,
                "Sign-in + policy",
                "also authorize each request with Rego (OPA)",
            )
            .item(
                GatewaySettings::DIRECT,
                "No gateway",
                "publish each service on its own host port",
            )
            .initial_value(current)
            .interact()
            .map_err(io_err)
    }

    /// Offer the recommended defaults first; customizing is opt-in.
    fn ask_mode() -> Result<Mode> {
        cliclack::select("How do you want to configure it?")
            .item(
                Mode::Defaults,
                "Use recommended defaults",
                "customize later in env.toml",
            )
            .item(Mode::Key, "Customize key settings", "auth, UIs, models")
            .item(
                Mode::All,
                "Customize everything",
                "also images and other advanced knobs",
            )
            .initial_value(Mode::Defaults)
            .interact()
            .map_err(io_err)
    }

    /// For each selected module, prompt the knobs `mode` covers — skipping any the
    /// CLI already set via `--set`, so a fully-scripted `--set` run asks nothing.
    /// Only answers that differ from a knob's default are recorded, so `env.toml`
    /// pins just what the user changed.
    fn configure_knobs(
        catalog: &Catalog,
        selected: &[ModuleId],
        overrides: &mut BTreeMap<ModuleId, BTreeMap<String, String>>,
        mode: Mode,
    ) -> Result<()> {
        if mode == Mode::Defaults {
            return Ok(());
        }
        for id in selected {
            let Some(module) = catalog.get(id) else {
                continue;
            };
            let unset = |k: &&Knob| !overrides.get(id).is_some_and(|m| m.contains_key(&k.key));
            let (key, advanced): (Vec<&Knob>, Vec<&Knob>) = module
                .knobs()
                .iter()
                .filter(unset)
                .partition(|k| !k.advanced);
            let offer_advanced = mode == Mode::All && !advanced.is_empty();
            if key.is_empty() && !offer_advanced {
                continue;
            }
            cliclack::log::step(format!("Configure {id}")).map_err(io_err)?;
            let mut asked = key;
            if offer_advanced {
                let keys: Vec<&str> = advanced.iter().map(|k| k.key.as_str()).collect();
                let tune = cliclack::confirm(format!(
                    "Change advanced settings for {id}? ({})",
                    keys.join(", ")
                ))
                .initial_value(false)
                .interact()
                .map_err(io_err)?;
                if tune {
                    asked.extend(advanced);
                }
            }
            for knob in asked {
                if let Some(help) = &knob.help {
                    cliclack::log::info(format!("  {help}")).map_err(io_err)?;
                }
                if let Some(value) = prompt_knob(id, knob)?
                    && !is_default(knob, &value)
                {
                    overrides
                        .entry(id.clone())
                        .or_default()
                        .insert(knob.key.clone(), value);
                }
            }
        }
        Ok(())
    }

    /// Whether a prompted `value` equals the knob's default (integers compared
    /// numerically; the other kinds are prompted in their canonical form).
    pub(super) fn is_default(knob: &Knob, value: &str) -> bool {
        let Some(default) = knob.default.as_deref() else {
            return false;
        };
        match knob.kind {
            KnobKind::Integer { .. } | KnobKind::Port => {
                match (default.trim().parse::<i64>(), value.trim().parse::<i64>()) {
                    (Ok(d), Ok(v)) => d == v,
                    _ => default == value,
                }
            }
            _ => default == value,
        }
    }

    /// Render the right `cliclack` control for a knob's [`KnobKind`] and return
    /// the chosen value as a string (the form the engine stores every knob in).
    /// Returns `None` when an optional knob is left blank.
    fn prompt_knob(module: &ModuleId, knob: &Knob) -> Result<Option<String>> {
        let label = knob.title.clone().unwrap_or_else(|| knob.key.clone());
        let prompt = format!("{module}.{label}");
        match &knob.kind {
            KnobKind::Bool => {
                let initial = knob.default.as_deref() == Some("true");
                let value = cliclack::confirm(prompt)
                    .initial_value(initial)
                    .interact()
                    .map_err(io_err)?;
                Ok(Some(value.to_string()))
            }
            KnobKind::Enum { options } => {
                let items: Vec<(String, String, String)> = options
                    .iter()
                    .map(|o| (o.clone(), o.clone(), String::new()))
                    .collect();
                let mut select = cliclack::select(prompt).items(&items);
                if let Some(default) = &knob.default {
                    select = select.initial_value(default.clone());
                }
                Ok(Some(select.interact().map_err(io_err)?))
            }
            KnobKind::Integer { min, max } => prompt_int(prompt, knob, *min, *max),
            KnobKind::Port => prompt_int(prompt, knob, Some(1), Some(65535)),
            KnobKind::String => {
                let mut input = cliclack::input(prompt);
                if let Some(default) = &knob.default {
                    input = input.default_input(default);
                }
                if !knob.required {
                    input = input.required(false);
                }
                let value: String = input.interact().map_err(io_err)?;
                Ok(non_empty(value))
            }
        }
    }

    /// A validated integer/port input honoring an optional inclusive range.
    fn prompt_int(
        prompt: String,
        knob: &Knob,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Result<Option<String>> {
        let mut input =
            cliclack::input(prompt).validate(move |s: &String| validate_int(s, min, max));
        if let Some(default) = &knob.default {
            input = input.default_input(default);
        }
        if !knob.required {
            input = input.required(false);
        }
        let value: String = input.interact().map_err(io_err)?;
        Ok(non_empty(value))
    }

    /// `cliclack` validator: the input parses as an `i64` within `[min, max]`.
    fn validate_int(
        s: &str,
        min: Option<i64>,
        max: Option<i64>,
    ) -> std::result::Result<(), String> {
        // Allow an empty string through so an optional knob can be left blank;
        // `required(false)` lets cliclack accept it, and `non_empty` drops it.
        if s.trim().is_empty() {
            return Ok(());
        }
        let n: i64 = s
            .trim()
            .parse()
            .map_err(|_| format!("`{s}` is not a whole number"))?;
        if let Some(lo) = min
            && n < lo
        {
            return Err(format!("must be at least {lo}"));
        }
        if let Some(hi) = max
            && n > hi
        {
            return Err(format!("must be at most {hi}"));
        }
        Ok(())
    }

    /// `Some(value)` when non-blank, else `None` (an optional knob left empty).
    fn non_empty(value: String) -> Option<String> {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use olai_stack_topology::{
        EnvManifest, GatewaySettings, KnobKind, ModuleId, PlanCtx, Selection, baseline_catalog,
    };

    use super::{
        config_summary, hosted_reference, knob_reference, next_steps, parse_env_lines, wizard,
    };

    #[test]
    fn env_lines_parse_like_dotenv() {
        let parsed = parse_env_lines(
            "# comment\n\nPG_PASSWORD=s3cr3t\nexport A = \"quoted value\"\nB='x=y'\nnot a pair\n",
        );
        assert_eq!(
            parsed,
            vec![
                ("PG_PASSWORD".to_string(), "s3cr3t".to_string()),
                ("A".to_string(), "quoted value".to_string()),
                ("B".to_string(), "x=y".to_string()),
            ]
        );
    }

    #[test]
    fn next_steps_point_at_the_files_a_user_needs() {
        let plain = baseline_catalog()
            .plan(
                &Selection::modules(["postgres", "mlflow"]),
                &PlanCtx {
                    gateway_host_port: 9090,
                    ..PlanCtx::default()
                },
            )
            .unwrap();
        let text = next_steps(&plain, std::path::Path::new("lh"));
        assert!(text.contains("http://localhost:9090"), "{text}");
        assert!(text.contains("client/host.env"), "{text}");
        assert!(text.contains("trestle env render lh"), "{text}");
        assert!(
            !text.contains("Sign in") && !text.contains("Policy") && !text.contains("Trust"),
            "{text}"
        );

        let guarded = baseline_catalog()
            .plan(
                &Selection {
                    gateway: GatewaySettings::POLICY,
                    ..Selection::modules(["postgres", "mlflow"])
                },
                &PlanCtx::default(),
            )
            .unwrap();
        let text = next_steps(&guarded, std::path::Path::new("lh"));
        assert!(text.contains("https://lakehouse.localhost:9080"), "{text}");
        assert!(
            text.contains("docker compose cp gateway-certs:/certs/ca.pem ."),
            "{text}"
        );
        assert!(text.contains("users.yml"), "{text}");
        assert!(text.contains("policies/authz.rego"), "{text}");
    }

    #[test]
    fn next_steps_point_at_the_graph_schema() {
        let plan = baseline_catalog()
            .plan(&Selection::modules(["puppygraph"]), &PlanCtx::default())
            .unwrap();
        let text = next_steps(&plan, std::path::Path::new("lh"));
        assert!(text.contains("graphs/puppygraph/schema.json"), "{text}");
        assert!(
            !next_steps(
                &baseline_catalog()
                    .plan(&lakehouse(), &PlanCtx::default())
                    .unwrap(),
                std::path::Path::new("lh")
            )
            .contains("Graph:")
        );
    }

    #[test]
    fn next_steps_point_at_the_pipelines() {
        let plan = baseline_catalog()
            .plan(&Selection::modules(["rivers"]), &PlanCtx::default())
            .unwrap();
        let text = next_steps(&plan, std::path::Path::new("lh"));
        assert!(text.contains("Pipelines: http://localhost:"), "{text}");
        assert!(text.contains("pipelines/rivers/"), "{text}");
    }

    fn lakehouse() -> Selection {
        Selection::modules(["postgres", "mlflow"])
    }

    #[test]
    fn every_image_knob_is_advanced() {
        for module in baseline_catalog().modules() {
            for knob in module.knobs() {
                if knob.key == "image" || knob.key.ends_with("_image") {
                    assert!(
                        knob.advanced,
                        "{}.{} should be advanced",
                        module.id(),
                        knob.key
                    );
                }
            }
        }
        let catalog = baseline_catalog();
        let headwaters = catalog.get(&ModuleId::from("headwaters")).unwrap();
        let serve_ui = headwaters
            .knobs()
            .iter()
            .find(|k| k.key == "serve_ui")
            .unwrap();
        assert!(!serve_ui.advanced, "headwaters.serve_ui is a key setting");
        // Sign-in is a gateway setting now, not a knob to tune.
        let envoy = catalog.get(&ModuleId::from("envoy")).unwrap();
        assert!(envoy.knobs().iter().all(|k| k.key != "auth"));
    }

    #[test]
    fn summary_lists_only_non_default_knobs() {
        let plan = |s: &Selection| baseline_catalog().plan(s, &PlanCtx::default()).unwrap();
        let dir = std::path::Path::new("lh");

        let defaults = lakehouse();
        let text = config_summary(&plan(&defaults), &defaults, dir);
        assert!(text.contains("Settings: all defaults"), "{text}");
        assert!(text.contains("Gateway:  open (no sign-in)"), "{text}");
        assert!(
            text.contains("envoy, ") || text.contains(", envoy"),
            "{text}"
        );
        assert!(text.contains("trestle env render lh"), "{text}");

        let mut tuned = lakehouse();
        tuned
            .knob_overrides
            .entry(ModuleId::from("postgres"))
            .or_default()
            .insert("image".into(), "postgres:17".into());
        tuned.gateway = GatewaySettings::POLICY;
        let text = config_summary(&plan(&tuned), &tuned, dir);
        assert!(text.contains("postgres.image = postgres:17"), "{text}");
        assert!(!text.contains("all defaults"), "{text}");
        assert!(
            text.contains("Gateway:  sign-in + policy (authelia, opa)"),
            "{text}"
        );
    }

    #[test]
    fn knob_reference_comments_out_defaults_and_stays_valid_toml() {
        let mut selection = lakehouse();
        selection.gateway = GatewaySettings::SIGN_IN;
        selection
            .knob_overrides
            .entry(ModuleId::from("envoy"))
            .or_default()
            .insert("image".into(), "envoy:pinned".into());
        let plan = baseline_catalog()
            .plan(&selection, &PlanCtx::default())
            .unwrap();
        let reference = knob_reference(&plan, &selection);

        // Untouched modules get a commented header; overridden ones only their remaining keys.
        assert!(
            reference.contains("# [selection.knob_overrides.postgres]\n# image = \"postgres:16\""),
            "{reference}"
        );
        assert!(
            !reference.contains("# [selection.knob_overrides.envoy]"),
            "{reference}"
        );
        assert!(
            reference.contains("# more for [selection.knob_overrides.envoy] above:"),
            "{reference}"
        );
        assert!(!reference.contains("# image = \"envoyproxy"), "{reference}");
        assert!(reference.contains("# certs_image = "), "{reference}");
        // Sign-in pulls in authelia; its knobs are listed too.
        assert!(
            reference.contains("[selection.knob_overrides.authelia]"),
            "{reference}"
        );
        assert!(
            reference
                .lines()
                .all(|l| l.is_empty() || l.starts_with('#')),
            "{reference}"
        );

        // Appended to the manifest it parses back to the same choices.
        let manifest = EnvManifest::new(selection.clone(), PlanCtx::default());
        let text = format!("{}{reference}", manifest.to_toml().unwrap());
        assert_eq!(EnvManifest::from_toml(&text).unwrap(), manifest);

        // Uncommenting a module's block yields a valid override table.
        let uncommented = reference
            .lines()
            .skip_while(|l| *l != "# [selection.knob_overrides.postgres]")
            .take(2)
            .map(|l| l.trim_start_matches("# "))
            .collect::<Vec<_>>()
            .join("\n");
        let edited =
            EnvManifest::from_toml(&format!("{}\n{uncommented}\n", manifest.to_toml().unwrap()))
                .unwrap();
        assert_eq!(
            edited.selection.knob_overrides[&ModuleId::from("postgres")]["image"],
            "postgres:16"
        );
    }

    #[test]
    fn hosted_reference_shows_the_default_connection_to_uncomment() {
        let selection = Selection::modules(["aws-s3", "unity-catalog"]);
        let plan = baseline_catalog()
            .plan(&selection, &PlanCtx::default())
            .unwrap();
        let reference = hosted_reference(&plan, &selection);
        assert!(
            reference.contains("# aws-s3 reads: AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY"),
            "{reference}"
        );
        assert!(
            reference
                .lines()
                .all(|l| l.is_empty() || l.starts_with('#')),
            "{reference}"
        );
        // Uncommented, it configures the provider with exactly its default.
        let manifest = EnvManifest::new(selection, PlanCtx::default());
        let uncommented: String = reference
            .lines()
            .filter(|l| l.starts_with("# [") || l.contains(" = "))
            .filter(|l| !l.starts_with("# ---"))
            .map(|l| format!("{}\n", l.trim_start_matches("# ")))
            .collect();
        let edited =
            EnvManifest::from_toml(&format!("{}\n{uncommented}", manifest.to_toml().unwrap()))
                .unwrap();
        assert_eq!(
            edited.selection.hosted[&ModuleId::from("aws-s3")],
            olai_stack_topology::HostedKind::S3.default_resource()
        );
        // A configured provider is not repeated.
        assert!(hosted_reference(&plan, &edited.selection).is_empty());
    }

    #[test]
    fn answers_equal_to_the_default_are_not_recorded() {
        let knob = |kind: KnobKind, default: &str| olai_stack_topology::Knob {
            key: "k".into(),
            title: None,
            kind,
            default: Some(default.into()),
            required: false,
            help: None,
            aliases: vec![],
            advanced: false,
        };
        assert!(wizard::is_default(&knob(KnobKind::Bool, "true"), "true"));
        assert!(!wizard::is_default(&knob(KnobKind::Bool, "true"), "false"));
        assert!(wizard::is_default(&knob(KnobKind::Port, "9080"), " 09080"));
        assert!(!wizard::is_default(
            &knob(KnobKind::String, "postgres:16"),
            "postgres:17"
        ));
    }
}
