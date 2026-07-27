//! `trestle env` — create and render composed Docker-Compose lakehouse
//! environments from the `olai-stack-topology` engine.
//!
//! This is the single, user-facing code path for producing composed environments,
//! kept deliberately separate from `trestle new`'s project scaffolding and
//! `trestle generate`'s codegen. Its three subcommands mirror the engine's phases:
//!
//! - [`EnvCommand::New`] — pick modules, set the config knobs each exposes, then
//!   plan + render into a target directory. Interactive (a [`cliclack`] wizard) or
//!   fully scripted (`--select` / `--set` / `--prefer`).
//! - [`EnvCommand::Render`] — re-render an existing environment from its persisted
//!   `env.toml` ([`EnvManifest`]); ports and routes stay stable by construction.
//! - [`EnvCommand::ListModules`] — enumerate the catalog and the knobs each module
//!   exposes.
//!
//! Every environment persists an editable `env.toml` alongside its rendered
//! artifacts, so re-rendering and hand-editing are the supported edit loop.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use olai_stack_topology::{
    Catalog, EnvManifest, Knob, KnobKind, ManifestError, ModuleId, PlanCtx, PlanError, Selection,
    baseline_catalog, baseline_selection, layout_report,
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

    /// Module ids to select, e.g. `--select envoy,postgres,mlflow`. Repeatable
    /// and comma-separated. When empty in interactive mode the wizard starts
    /// from the default lakehouse selection.
    #[clap(long = "select", value_name = "MODULE[,MODULE]", value_delimiter = ',')]
    pub select: Vec<String>,

    /// Knob override, `module.key=value` (e.g. `--set envoy.auth=true`).
    /// Repeatable. A value set here is not asked for again by the wizard.
    #[clap(long = "set", value_name = "MODULE.KEY=VALUE", value_parser = parse_knob_override)]
    pub knobs: Vec<((String, String), String)>,

    /// Prefer a provider for a resource role, `role=provider[,provider...]`
    /// (e.g. `--prefer object_store=azurite,seaweedfs`). Repeatable.
    #[clap(long = "prefer", value_name = "ROLE=PROVIDER[,PROVIDER]", value_parser = parse_selection)]
    pub prefer: Vec<(String, Vec<String>)>,

    /// Gateway host-published port (default 9080).
    #[clap(long)]
    pub gateway_host_port: Option<u16>,

    /// Root data directory injected as `${DATA_ROOT}` (default `./.data`).
    #[clap(long)]
    pub data_root: Option<String>,

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

    // Seed the selection + knob overrides from CLI flags. When nothing was
    // selected and we're interactive, start from the default lakehouse so the
    // wizard has sensible pre-checks.
    let mut selection = if args.select.is_empty() && !args.non_interactive {
        baseline_selection()
    } else {
        Selection::modules(args.select.iter().cloned())
    };
    for ((module, knob), value) in &args.knobs {
        selection
            .knob_overrides
            .entry(ModuleId::from(module.as_str()))
            .or_default()
            .insert(knob.clone(), value.clone());
    }

    // The context carries the environment-level facts the planner can't derive.
    let mut ctx = PlanCtx {
        env_name: args.name.clone(),
        ..PlanCtx::default()
    };
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

    // Interactive: let the wizard pick modules and fill each knob the CLI didn't.
    if !args.non_interactive {
        wizard::run(&catalog, &mut selection)?;
    }

    // Plan, then materialize + persist. This is the shared core with `env render`.
    let plan = catalog.plan(&selection, &ctx).map_err(map_plan_err)?;

    if args.non_interactive {
        println!("{}", layout_report(&plan));
    } else {
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
    report_written(&out_dir);
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
    plan.materialize()
        .write_to(dir)
        .map_err(|e| Error::io_at(dir, e))?;
    let manifest = EnvManifest::new(selection.clone(), ctx.clone());
    let manifest_path = dir.join("env.toml");
    manifest
        .write_to(&manifest_path)
        .map_err(map_manifest_err)?;
    Ok(())
}

fn report_written(dir: &std::path::Path) {
    let dir = dir.display();
    println!("Wrote composed environment to {dir}");
    println!("Next steps:");
    println!("  cd {dir}");
    println!("  docker compose up");
}

// ---------------------------------------------------------------------------
// env render
// ---------------------------------------------------------------------------

fn run_render(args: EnvRenderArgs) -> Result<()> {
    let manifest_path = args.dir.join("env.toml");
    let manifest = EnvManifest::read_from(&manifest_path).map_err(|e| {
        map_manifest_err(e).with_hint(format!(
            "expected an `env.toml` in {} — create one with `trestle env new`",
            args.dir.display()
        ))
    })?;

    let catalog = baseline_catalog();
    let plan = manifest.plan(&catalog).map_err(map_plan_err)?;

    let out_dir = args.out_dir.clone().unwrap_or_else(|| args.dir.clone());
    // The manifest dir is expected to be non-empty (it holds env.toml); only
    // guard a *different* output dir against clobbering unrelated files.
    if out_dir != args.dir {
        ensure_writable_dir(&out_dir, args.force)?;
    }

    println!("{}", layout_report(&plan));
    plan.materialize()
        .write_to(&out_dir)
        .map_err(|e| Error::io_at(&out_dir, e))?;
    report_written(&out_dir);
    Ok(())
}

// ---------------------------------------------------------------------------
// env list-modules
// ---------------------------------------------------------------------------

fn run_list_modules(args: EnvListModulesArgs) -> Result<()> {
    let catalog = baseline_catalog();
    for module in catalog.modules() {
        let id = module.id();
        let name = module.display_name().unwrap_or(id.as_str());
        let category = module
            .category()
            .map(|c| format!("  [{c}]"))
            .unwrap_or_default();
        match module.summary() {
            Some(summary) => println!("{id}  {name} — {summary}{category}"),
            None => println!("{id}  {name}{category}"),
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

fn map_plan_err(e: PlanError) -> Error {
    Error::other(format!("planning the environment failed: {e}"))
        .with_hint("run `trestle env list-modules` to see valid module ids and knobs")
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
    use super::{Catalog, Knob, KnobKind, ModuleId, Result, Selection, io_err};
    use std::collections::BTreeMap;

    /// Drive the interactive create flow, mutating `selection` in place: pick the
    /// module set, then fill each knob the CLI didn't already set via `--set`.
    pub fn run(catalog: &Catalog, selection: &mut Selection) -> Result<()> {
        cliclack::intro("trestle env new").map_err(io_err)?;

        selection.modules = ask_modules(catalog, &selection.modules)?;
        configure_knobs(
            catalog,
            &selection.modules.clone(),
            &mut selection.knob_overrides,
        )?;

        Ok(())
    }

    /// Multi-select the module set, pre-checking `preselected`.
    fn ask_modules(catalog: &Catalog, preselected: &[ModuleId]) -> Result<Vec<ModuleId>> {
        let items: Vec<(String, String, String)> = catalog
            .modules()
            .iter()
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
        Ok(picked.into_iter().map(ModuleId::from).collect())
    }

    /// For each selected module, prompt every knob it exposes — skipping any the
    /// CLI already set via `--set`, so a fully-scripted `--set` run asks nothing.
    fn configure_knobs(
        catalog: &Catalog,
        selected: &[ModuleId],
        overrides: &mut BTreeMap<ModuleId, BTreeMap<String, String>>,
    ) -> Result<()> {
        for id in selected {
            let Some(module) = catalog.get(id) else {
                continue;
            };
            let knobs = module.knobs();
            if knobs.is_empty() {
                continue;
            }
            cliclack::log::step(format!("Configure {id}")).map_err(io_err)?;
            for knob in knobs {
                if overrides.get(id).is_some_and(|m| m.contains_key(&knob.key)) {
                    cliclack::log::info(format!("  {}.{} → (set via --set)", id, knob.key))
                        .map_err(io_err)?;
                    continue;
                }
                if let Some(help) = &knob.help {
                    cliclack::log::info(format!("  {help}")).map_err(io_err)?;
                }
                if let Some(value) = prompt_knob(id, knob)? {
                    overrides
                        .entry(id.clone())
                        .or_default()
                        .insert(knob.key.clone(), value);
                }
            }
        }
        Ok(())
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
