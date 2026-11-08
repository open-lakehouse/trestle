//! The store-backed `trestle env` verbs: environments of a workspace in the trestle state
//! store, edited as a draft and committed as numbered, immutable revisions.
//!
//! `env new` / `env render` / `env diagram` keep working on directories; these take a
//! `<ws>/<env>[@<rev>]` target instead.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use clap::Args;
use olai_stack_runtime::MaskedVar;
use olai_stack_state::{Draft, Environment, Revision, Store, VarSet, VarValue, read_env_dir};
use olai_stack_topology::{
    Catalog, EnvManifest, ModuleId, PlanCtx, Selection, baseline_catalog, baseline_selection,
};
use serde::Serialize;

use super::env::{map_plan_err, parse_knob_override};
use super::output::emit;
use super::state::{Ctx, environment_keychain_refs, with_instances_hint};
use super::target::{Rev, Target, revision};
use super::util::ensure_writable_dir;
use super::var::{VarRow, var_rows};
use super::ws::remove_instances;
use crate::error::{Error, Result};

#[derive(Args, Clone)]
pub struct CreateArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// Start from this `env.toml`, or an environment directory holding one (with the module
    /// manifests it lists). Defaults to the default lakehouse selection.
    #[clap(long, conflicts_with = "select")]
    pub from: Option<PathBuf>,

    /// Module ids to select instead of the default lakehouse, e.g. `--select postgres,mlflow`.
    #[clap(long = "select", value_name = "MODULE[,MODULE]", value_delimiter = ',')]
    pub select: Vec<String>,
}

#[derive(Args, Clone)]
pub struct ShowArgs {
    /// `<ws>/<env>` (the draft), `<ws>/<env>@<n>` or `<ws>/<env>@head`.
    pub target: Target,

    /// Show the environment an instance of the revision runs with: every variable with the
    /// layer it comes from, secrets masked.
    #[clap(long)]
    pub effective: bool,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct ModulesArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// Module ids.
    #[clap(required = true, value_delimiter = ',')]
    pub modules: Vec<String>,
}

#[derive(Args, Clone)]
pub struct SetKnobArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// `module.key=value`.
    #[clap(required = true, value_parser = parse_knob_override)]
    pub knobs: Vec<((String, String), String)>,
}

#[derive(Args, Clone)]
pub struct CommitArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// What the revision changes, shown by `env revisions` and the editor's history.
    #[clap(short, long)]
    pub message: Option<String>,
}

#[derive(Args, Clone)]
pub struct RenameArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// The new name: lowercase letters, digits and dashes.
    pub name: String,
}

#[derive(Args, Clone)]
pub struct RevisionsArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct DiffArgs {
    /// `<ws>/<env>[@<from>]`: from the head revision unless a revision is given.
    pub target: Target,

    /// What to compare against: a revision number, `head` or `draft` (the default).
    #[clap(long, default_value = "draft")]
    pub to: String,
}

#[derive(Args, Clone)]
pub struct ExportArgs {
    /// `<ws>/<env>[@<rev>]` (the head revision by default).
    pub target: Target,

    /// The directory to write.
    pub dir: PathBuf,

    /// Write into a non-empty directory (overwrites the exported files).
    #[clap(long)]
    pub force: bool,
}

#[derive(Args, Clone)]
pub struct RmArgs {
    /// `<ws>/<env>`.
    pub target: Target,

    /// Stop and remove its running instances first (with their volumes).
    #[clap(long)]
    pub force: bool,
}

/// The catalog a draft or revision plans against: the baseline with its module manifests
/// merged over it.
pub(super) fn catalog_for(modules: &BTreeMap<String, String>) -> Result<Catalog> {
    let mut catalog = baseline_catalog();
    for (path, text) in modules {
        let custom = Catalog::from_manifests([text.as_str()])
            .map_err(|e| Error::other(format!("module manifest `{path}`: {e}")))?;
        catalog = catalog.merge(custom);
    }
    Ok(catalog)
}

/// Check that `draft` plans (without secrets, so hosted references stay as they are).
fn validate(draft: &Draft) -> Result<()> {
    draft
        .manifest
        .plan(&catalog_for(&draft.modules)?)
        .map(drop)
        .map_err(map_plan_err)
}

pub(super) fn run_create(args: CreateArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    args.target.no_rev()?;
    let ws = args.target.workspace(&ctx.store)?;
    let name = args
        .target
        .env
        .clone()
        .ok_or_else(|| Error::other("`env create` needs <ws>/<env>"))?;
    let draft = match &args.from {
        Some(path) if path.is_dir() => read_env_dir(path)?,
        Some(path) => Draft::new(
            EnvManifest::read_from(path)
                .map_err(|e| Error::other(format!("reading {}: {e}", path.display())))?,
        ),
        None => {
            let selection = if args.select.is_empty() {
                baseline_selection()
            } else {
                Selection::modules(args.select.iter().cloned())
            };
            Draft::new(EnvManifest::new(selection, PlanCtx::default()))
        }
    };
    validate(&draft)?;
    ctx.store.create_environment(&ws.id, &name, draft)?;
    println!(
        "created {}/{name} (a draft; nothing committed yet)",
        ws.name
    );
    println!("  next: `trestle env commit {}/{name}`", ws.name);
    Ok(())
}

/// Change the target's draft with `f`, check it still plans, and write it.
fn edit(target: &Target, f: impl FnOnce(&mut Draft) -> Result<()>) -> Result<()> {
    let ctx = Ctx::open()?;
    target.no_rev()?;
    let (_, env) = target.environment(&ctx.store)?;
    ctx.store.modify_draft(&env.id, |draft| {
        f(draft)?;
        validate(draft)
    })?;
    println!("updated the draft of {target}");
    Ok(())
}

pub(super) fn run_add_module(args: ModulesArgs) -> Result<()> {
    edit(&args.target, |draft| {
        for m in &args.modules {
            draft.manifest.add_module(m.as_str());
        }
        Ok(())
    })
}

pub(super) fn run_remove_module(args: ModulesArgs) -> Result<()> {
    edit(&args.target, |draft| {
        for m in &args.modules {
            let id = ModuleId::from(m.as_str());
            if !draft.manifest.selection.modules.contains(&id) {
                return Err(Error::other(format!("`{m}` is not selected")));
            }
            draft.manifest.remove_module(&id);
        }
        Ok(())
    })
}

pub(super) fn run_set_knob(args: SetKnobArgs) -> Result<()> {
    edit(&args.target, |draft| {
        for ((module, key), value) in &args.knobs {
            draft
                .manifest
                .set_knob(module.as_str(), key.as_str(), value.as_str());
        }
        Ok(())
    })
}

/// `manifest` as `env.toml` text, under the environment's current name.
fn toml_as(env: &Environment, manifest: &EnvManifest) -> Result<String> {
    let mut manifest = manifest.clone();
    manifest.context.env_name = env.name.clone();
    manifest
        .to_toml()
        .map_err(|e| Error::other(format!("serializing the manifest: {e}")))
}

/// The `env.toml` text a draft commits as.
fn draft_toml(env: &Environment) -> Result<String> {
    toml_as(env, &env.draft.manifest)
}

/// A revision's `env.toml` text under the environment's current name. A revision keeps the
/// `env_name` it was committed with, but an instance is planned under the current one, so a
/// rename alone changes nothing a diff or a commit should show.
fn revision_toml(env: &Environment, rev: &Revision) -> Result<String> {
    if rev.manifest().context.env_name == env.name {
        return Ok(rev.manifest_toml().to_owned());
    }
    toml_as(env, rev.manifest())
}

/// Whether the environment's draft would commit as something other than `rev`.
pub(super) fn draft_differs(env: &Environment, rev: &Revision) -> Result<bool> {
    Ok(draft_toml(env)? != revision_toml(env, rev)?
        || &env.draft.modules != rev.modules()
        || &env.draft.vars != rev.vars())
}

pub(super) fn run_commit(args: CommitArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    args.target.no_rev()?;
    let (ws, env) = args.target.environment(&ctx.store)?;
    if let Some(head) = env.head {
        let head = ctx.store.revision(&env.id, head)?;
        if !draft_differs(&env, &head)? {
            println!(
                "nothing to commit: the draft of {}/{} is r{}",
                ws.name,
                env.name,
                head.number()
            );
            return Ok(());
        }
    }
    let rev = ctx
        .store
        .commit_draft_with(&env.id, args.message.as_deref())?;
    println!(
        "committed {}/{} r{} ({})",
        ws.name,
        env.name,
        rev.number(),
        &rev.digest()[..12]
    );
    Ok(())
}

pub(super) fn run_rename(args: RenameArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    args.target.no_rev()?;
    let (ws, env) = args.target.environment(&ctx.store)?;
    let from = format!("{}/{}", ws.name, env.name);
    ctx.store
        .rename_environment(&env.id, &args.name)
        .map_err(|e| with_instances_hint(e, &from))?;
    println!("renamed {from} to {}/{}", ws.name, args.name);
    Ok(())
}

/// A revision in `env revisions`.
#[derive(Serialize)]
struct RevRow {
    number: u32,
    digest: String,
    head: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// When it was committed (RFC 3339, UTC).
    created_at: chrono::DateTime<chrono::Utc>,
    modules: Vec<String>,
    vars: usize,
    instances: usize,
}

pub(super) fn run_revisions(args: RevisionsArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let (_, env) = args.target.environment(&ctx.store)?;
    let instances = ctx.store.instances_of(&env.id)?;
    let rows: Vec<RevRow> = ctx
        .store
        .revisions(&env.id)?
        .into_iter()
        .map(|r| RevRow {
            number: r.number(),
            digest: r.digest().to_owned(),
            head: env.head == Some(r.number()),
            message: r.message().map(str::to_owned),
            created_at: r.created_at().into(),
            modules: module_names(r.manifest()),
            vars: r.vars().0.len(),
            instances: instances
                .iter()
                .filter(|i| i.revision == r.number())
                .count(),
        })
        .collect();
    emit(args.json, "revisions", &rows, |rows| {
        if rows.is_empty() {
            println!("no revisions; `trestle env commit {}`", args.target);
        }
        for r in rows {
            println!(
                "{} r{:<3} {}  {}  {}{}",
                if r.head { "*" } else { " " },
                r.number,
                &r.digest[..12],
                r.created_at.format("%Y-%m-%d %H:%M"),
                r.modules.join(","),
                match r.instances {
                    0 => String::new(),
                    n => format!("  ({n} instance{})", if n == 1 { "" } else { "s" }),
                }
            );
            if let Some(message) = &r.message {
                println!("      {message}");
            }
        }
    })
}

fn module_names(manifest: &EnvManifest) -> Vec<String> {
    manifest
        .selection
        .modules
        .iter()
        .map(|m| m.to_string())
        .collect()
}

/// What an environment's draft or a revision holds, as text to diff.
struct Texts {
    label: String,
    manifest: String,
    modules: BTreeMap<String, String>,
    vars: String,
}

fn vars_text(vars: &VarSet) -> String {
    vars.0
        .iter()
        .map(|(k, v)| match v {
            VarValue::Literal(l) => format!("{k}={l}\n"),
            VarValue::Secret(r) => format!("{k}=<secret {}:{}>\n", r.provider, r.key),
        })
        .collect()
}

fn texts(store: &Store, env: &Environment, rev: Rev) -> Result<Texts> {
    Ok(match rev {
        Rev::Draft => Texts {
            label: "draft".into(),
            manifest: draft_toml(env)?,
            modules: env.draft.modules.clone(),
            vars: vars_text(&env.draft.vars),
        },
        rev => {
            let r = revision(store, env, Some(rev))?;
            Texts {
                label: format!("r{}", r.number()),
                manifest: revision_toml(env, &r)?,
                modules: r.modules().clone(),
                vars: vars_text(r.vars()),
            }
        }
    })
}

/// A unified diff of `a` → `b`, labelled `path`; empty when they are equal.
fn unified(path: &str, (la, a): (&str, &str), (lb, b): (&str, &str)) -> String {
    if a == b {
        return String::new();
    }
    similar::TextDiff::from_lines(a, b)
        .unified_diff()
        .header(&format!("{la}/{path}"), &format!("{lb}/{path}"))
        .to_string()
}

pub(super) fn run_diff(args: DiffArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let (_, env) = args.target.environment(&ctx.store)?;
    let to = match args.to.as_str() {
        "draft" => Rev::Draft,
        "head" => Rev::Head,
        n => Rev::Number(n.parse().map_err(|_| {
            Error::other(format!(
                "--to: expected a revision, `head` or `draft`, got `{n}`"
            ))
        })?),
    };
    let from = texts(&ctx.store, &env, args.target.rev.unwrap_or(Rev::Head))?;
    let to = texts(&ctx.store, &env, to)?;
    let (la, lb) = (from.label.as_str(), to.label.as_str());
    let mut out = unified("env.toml", (la, &from.manifest), (lb, &to.manifest));
    let paths: std::collections::BTreeSet<&String> =
        from.modules.keys().chain(to.modules.keys()).collect();
    for path in paths {
        let a = from.modules.get(path).map_or("", String::as_str);
        let b = to.modules.get(path).map_or("", String::as_str);
        out += &unified(path, (la, a), (lb, b));
    }
    out += &unified("vars", (la, &from.vars), (lb, &to.vars));
    if out.is_empty() {
        println!("{la} and {lb} are the same");
    } else {
        print!("{out}");
    }
    Ok(())
}

pub(super) fn run_export(args: ExportArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let (_, env) = args.target.environment(&ctx.store)?;
    let rev = revision(&ctx.store, &env, args.target.rev)?;
    ensure_writable_dir(&args.dir, args.force)?;
    ctx.store
        .export_revision(&env.id, rev.number(), &args.dir)?;
    println!(
        "exported {}/{}@{} to {}",
        args.target.workspace,
        env.name,
        rev.number(),
        args.dir.display()
    );
    Ok(())
}

pub(super) fn run_rm(args: RmArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    args.target.no_rev()?;
    let (ws, env) = args.target.environment(&ctx.store)?;
    let instances = ctx.store.instances_of(&env.id)?;
    ctx.block_on(remove_instances(
        &ctx,
        &instances,
        args.force,
        &format!("environment `{}/{}`", ws.name, env.name),
    ))?;
    let refs = environment_keychain_refs(&ctx.store, &env)?;
    ctx.store.delete_environment(&env.id)?;
    for r in refs {
        ctx.secrets().delete(&r.key)?;
    }
    println!("deleted {}/{}", ws.name, env.name);
    Ok(())
}

/// An environment's draft or revision in `env show`.
#[derive(Serialize)]
struct EnvDetail {
    workspace: String,
    name: String,
    /// `draft`, or the revision number.
    revision: String,
    head: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
    modules: Vec<String>,
    knobs: BTreeMap<String, BTreeMap<String, String>>,
    module_manifests: Vec<String>,
    vars: Vec<VarRow>,
}

/// The variables an instance of a revision runs with, in `env show --effective`.
#[derive(Serialize)]
struct EffectiveDetail {
    workspace: String,
    name: String,
    revision: u32,
    instance: String,
    vars: BTreeMap<String, MaskedVar>,
}

pub(super) fn run_show(args: ShowArgs) -> Result<()> {
    let ctx = Ctx::open()?;
    let (ws, env) = args.target.environment(&ctx.store)?;
    if args.effective {
        let rev = revision(&ctx.store, &env, args.target.rev)?;
        let detail = ctx.block_on(effective(&ctx, &env, rev.number()))?;
        let detail = EffectiveDetail {
            workspace: ws.name,
            name: env.name,
            ..detail
        };
        return emit(args.json, "effective_env", &detail, |d| {
            println!("{}/{}@{} ({})", d.workspace, d.name, d.revision, d.instance);
            for (k, v) in &d.vars {
                println!("{k}={}  [{}]", v.value, layer_name(&v.provenance));
            }
        });
    }
    let (revision_label, digest, manifest, modules, vars) = match args.target.rev {
        None | Some(Rev::Draft) => (
            "draft".to_owned(),
            None,
            env.draft.manifest.clone(),
            env.draft.modules.keys().cloned().collect(),
            var_rows(&env.draft.vars, "draft"),
        ),
        rev => {
            let r = revision(&ctx.store, &env, rev)?;
            (
                r.number().to_string(),
                Some(r.digest().to_owned()),
                r.manifest().clone(),
                r.modules().keys().cloned().collect(),
                var_rows(r.vars(), "revision"),
            )
        }
    };
    let detail = EnvDetail {
        workspace: ws.name,
        name: env.name.clone(),
        revision: revision_label,
        head: env.head,
        digest,
        modules: module_names(&manifest),
        knobs: manifest
            .selection
            .knob_overrides
            .iter()
            .map(|(m, k)| (m.to_string(), k.clone().into_iter().collect()))
            .collect(),
        module_manifests: modules,
        vars,
    };
    emit(args.json, "environment", &detail, |d| {
        let head = d.head.map_or("none".to_owned(), |h| format!("r{h}"));
        println!("{}/{} ({}; head {head})", d.workspace, d.name, d.revision);
        println!("modules: {}", d.modules.join(", "));
        for (module, knobs) in &d.knobs {
            for (k, v) in knobs {
                println!("  {module}.{k} = {v}");
            }
        }
        for m in &d.module_manifests {
            println!("module manifest: {m}");
        }
        for v in &d.vars {
            println!("var {v}");
        }
    })
}

fn layer_name(layer: &olai_stack_runtime::Layer) -> String {
    use olai_stack_runtime::Layer;
    match layer {
        Layer::CatalogDefaults => "catalog defaults".into(),
        Layer::RevisionManifest => "revision manifest".into(),
        Layer::Placement => "placement".into(),
        Layer::ClientEnv => "client env".into(),
        Layer::WorkspaceVars => "workspace vars".into(),
        Layer::EnvironmentVars => "environment vars".into(),
        Layer::RevisionVars => "revision vars".into(),
        Layer::Source(name) => format!("source:{name}"),
        Layer::InstanceOverrides => "instance overrides".into(),
    }
}

/// The effective environment of revision `number`: that of its instance, or of a throwaway one
/// (deleted again, releasing the slot it leased) when it has none.
async fn effective(ctx: &Ctx, env: &Environment, number: u32) -> Result<EffectiveDetail> {
    let existing = ctx
        .store
        .instances_of(&env.id)?
        .into_iter()
        .filter(|i| i.revision == number)
        .max_by_key(|i| i.desired == olai_stack_state::Desired::Running);
    let (id, throwaway) = match existing {
        Some(i) => (i.id, false),
        None => (ctx.store.create_instance(&env.id, number)?.id, true),
    };
    let resolved = ctx.instances().effective_env(&id).await;
    if throwaway {
        ctx.store.delete_instance(&id)?;
    }
    let mut vars = resolved?.masked();
    // The throwaway instance's id and slot mean nothing once it is gone.
    if throwaway {
        for key in ["TRESTLE_INSTANCE", "TRESTLE_SLOT"] {
            vars.remove(key);
        }
    }
    let mut instance = String::new();
    if throwaway {
        instance.push_str("no instance");
    } else {
        let _ = write!(instance, "{id}");
    }
    Ok(EffectiveDetail {
        workspace: String::new(),
        name: String::new(),
        revision: number,
        instance,
        vars,
    })
}
