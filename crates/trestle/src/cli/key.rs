//! `trestle key`: the keyrings of the keys modules seal their data under through the secret
//! server's key service (`Provides::keys`).
//!
//! A keyring is a key's versioned KEKs, which the secret server alone holds: the module only
//! asks it to wrap and unwrap its data keys. It lives in this machine's keychain, as
//! `trestle var set --secret` would store it, under the variable the planner derives for the
//! key (`TRESTLE_KEYRING_<MODULE>__<KEY>`), on the environment.
//!
//! - `import`: version 1 from an existing 32-byte key (stdin, or `--from <VAR>`), e.g.
//!   `UC_SERVER_KEK` with `--alias kek-1`.
//! - `rotate`: a new random version, made active (version 1 when there is no keyring yet).
//! - `retire --below <n> --confirm-swept`: raise `min_decrypt`, once the consumer's sweep has
//!   moved every blob off the lower versions.
//! - `show`: versions, active, `min_decrypt` and aliases; never material.
//!
//! Writes are **add-only**: none removes a version's material, and a write that would (the
//! stored keyring changed underneath, say) is refused. Each write reads the keyring back and
//! checks it. After a write, the running instances' `secret-server` is recreated with it, as
//! `trestle secret rotate` does; the module keeps running and its key service client picks the
//! change up.
//!
//! The keychain holds the only copy of a keyring: lose it and the data sealed under it can't be
//! read. On a team's state each machine has its own keychain, so the writes are refused there.
//!
//! `trestle up` checks first ([`ensure_keyrings`]) that every key the revision seals under has a
//! keyring. A key whose module sealed under a key of its own before (`Provides::key_imports`,
//! e.g. `unity-catalog-rs`'s `UC_SERVER_KEK`) is imported from it once, as `import` would;
//! otherwise `up` stops and says what to run. On a team's state it refuses such a revision.

use std::io::IsTerminal;

use clap::{Args, Subcommand};
use olai_stack_runtime::{
    EnvProvider, Keyring, KeyringSummary, SecretProvider, SecretProviders, SecretValue, kek_id,
};
use olai_stack_state::{Environment, Revision, VarValue};
use olai_stack_topology::plan::keyring_var;
use olai_stack_topology::{KeyImport, ModuleId};
use serde::Serialize;

use super::output::emit;
use super::secret::{recreate_secret_server, running_instances};
use super::state::{Ctx, KEYCHAIN};
use super::target::{Rev, Target, revision};
use crate::error::{Error, Result};

/// The `trestle key` subcommand group.
#[derive(Subcommand)]
pub enum KeyCommand {
    /// Make version 1 of a key's keyring from an existing 32-byte key (standard base64), read
    /// from stdin or from an existing secret (`--from`).
    Import(ImportArgs),
    /// Add a new random version to a key's keyring and make it active (version 1 when it has
    /// none yet), then recreate the secret server of the running instances.
    Rotate(KeyArgs),
    /// Stop unwrapping versions below `--below`, once the consumer's sweep reports no blob
    /// under them. Their material stays.
    Retire(RetireArgs),
    /// Show a key's versions, active version, `min_decrypt` and aliases (never material).
    Show(ShowArgs),
}

#[derive(Args, Clone)]
pub struct KeyArgs {
    /// `<ws>/<env>` (its running instances) or `<ws>/<env>@<n>` (revision n's).
    pub target: Target,

    /// The key: `<module>/<key>`, as the module declares it.
    pub key: KeyName,
}

#[derive(Args, Clone)]
pub struct ImportArgs {
    #[clap(flatten)]
    pub key: KeyArgs,

    /// Read the key from this existing secret of the environment (its environment or
    /// workspace vars, else this process's environment), e.g. `UC_SERVER_KEK`.
    #[clap(long, value_name = "VAR")]
    pub from: Option<String>,

    /// Keep the id the key had before (e.g. `kek-1` for a `LocalKeyProvider` KEK) as an alias
    /// of version 1, so what it sealed still opens.
    #[clap(long)]
    pub alias: Option<String>,
}

#[derive(Args, Clone)]
pub struct RetireArgs {
    #[clap(flatten)]
    pub key: KeyArgs,

    /// The new `min_decrypt`: versions below it no longer unwrap. At most the active version.
    #[clap(long, value_name = "N")]
    pub below: u32,

    /// Confirm the consumer's rotation sweep (olai-store's `rewrap_all`, then `kek_usage`)
    /// reports no blob under a version below N.
    #[clap(long)]
    pub confirm_swept: bool,
}

#[derive(Args, Clone)]
pub struct ShowArgs {
    #[clap(flatten)]
    pub key: KeyArgs,

    /// Print a JSON document (`kind: "keyring"`).
    #[clap(long)]
    pub json: bool,
}

/// The prefix of every keyring's variable (see `keyring_var`).
const KEYRING_PREFIX: &str = "TRESTLE_KEYRING_";

/// Whether `var` holds a keyring, which only `trestle key` writes.
pub(super) fn is_keyring_var(var: &str) -> bool {
    var.starts_with(KEYRING_PREFIX)
}

/// Refuse to write or remove the keyring `var` other than through `trestle key`: those writes
/// would bypass the add-only check.
pub(super) fn refuse_keyring_var(var: &str, what: &str) -> Result<()> {
    if is_keyring_var(var) {
        return Err(Error::other(format!(
            "`{var}` holds a keyring, which `{what}` can't change: keyrings are add-only, and \
             only `trestle key` writes them"
        ))
        .with_hint("`trestle key rotate|retire <ws>/<env> <module>/<key>`"));
    }
    Ok(())
}

/// Split `refs` into those to delete and the keyrings' entries, which are kept: removing an
/// environment never deletes the only copy of a key its data may be sealed under.
pub(super) fn keep_keyrings(
    refs: Vec<olai_stack_state::SecretRef>,
) -> (
    Vec<olai_stack_state::SecretRef>,
    Vec<olai_stack_state::SecretRef>,
) {
    refs.into_iter().partition(|r| {
        !r.key
            .rsplit_once('/')
            .is_some_and(|(_, var)| is_keyring_var(var))
    })
}

/// Print the keychain entries `keep_keyrings` kept.
pub(super) fn report_kept(kept: &[olai_stack_state::SecretRef]) {
    for r in kept {
        println!(
            "kept the keyring {} in the keychain (service `trestle`): delete it yourself once \
             nothing sealed under it is needed",
            r.key
        );
    }
}

/// A key as the CLI names it: `<module>/<key>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyName {
    pub module: ModuleId,
    pub key: String,
}

impl std::str::FromStr for KeyName {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.split_once('/') {
            Some((module, key)) if !module.is_empty() && !key.is_empty() && !key.contains('/') => {
                Ok(KeyName {
                    module: ModuleId::from(module),
                    key: key.to_owned(),
                })
            }
            _ => Err(format!("expected <module>/<key>, got `{s}`")),
        }
    }
}

impl std::fmt::Display for KeyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.module, self.key)
    }
}

pub fn run(cmd: KeyCommand) -> Result<()> {
    let ctx = Ctx::open()?;
    match cmd {
        KeyCommand::Import(args) => run_import(&ctx, args),
        KeyCommand::Rotate(args) => run_rotate(&ctx, args),
        KeyCommand::Retire(args) => run_retire(&ctx, args),
        KeyCommand::Show(args) => run_show(&ctx, args),
    }
}

/// What a command works on: the environment, the key's variable, and the running instances.
struct Resolved {
    env_target: Target,
    env: Environment,
    var: String,
    running: Vec<olai_stack_state::Instance>,
}

fn resolve(ctx: &Ctx, args: &KeyArgs) -> Result<Resolved> {
    if args.target.env.is_none() {
        return Err(Error::other(
            "a key's keyring belongs to an environment (<ws>/<env>)",
        ));
    }
    if args.target.rev == Some(Rev::Draft) {
        return Err(Error::other("the draft doesn't run; use <ws>/<env>"));
    }
    let (_, env) = args.target.environment(&ctx.store)?;
    let running = running_instances(ctx, &args.target)?;
    let var = check_served(ctx, &args.target, &env, &running, &args.key)?;
    Ok(Resolved {
        env_target: Target {
            rev: None,
            ..args.target.clone()
        },
        env,
        var,
        running,
    })
}

/// The key must be one a module declares in a running instance's revision, or (when none
/// runs) in the revision the target names. Returns its keyring's variable.
fn check_served(
    ctx: &Ctx,
    target: &Target,
    env: &Environment,
    running: &[olai_stack_state::Instance],
    key: &KeyName,
) -> Result<String> {
    let mut numbers: Vec<u32> = running.iter().map(|i| i.revision).collect();
    if numbers.is_empty() {
        numbers.push(revision(&ctx.store, env, target.rev)?.number());
    }
    numbers.sort_unstable();
    numbers.dedup();
    let mut served = Vec::new();
    for number in &numbers {
        let rev = revision(&ctx.store, env, Some(Rev::Number(*number)))?;
        served.extend(served_keys(&rev)?);
    }
    if served.contains(key) {
        return Ok(keyring_var(&key.module, &key.key));
    }
    let revs: Vec<String> = numbers.iter().map(|n| format!("r{n}")).collect();
    served.sort_by_key(ToString::to_string);
    served.dedup();
    let known = if served.is_empty() {
        "no module there declares a key".to_owned()
    } else {
        let names: Vec<String> = served.iter().map(ToString::to_string).collect();
        format!("its keys: {}", names.join(", "))
    };
    Err(Error::other(format!(
        "`{key}` is not a key of {target} ({})",
        revs.join(", ")
    ))
    .with_hint(known))
}

/// The keys the revision's secret server holds keyrings for.
fn served_keys(rev: &Revision) -> Result<Vec<KeyName>> {
    Ok(served_key_imports(rev)?
        .into_iter()
        .map(|(key, _)| key)
        .collect())
}

/// [`served_keys`], each with where its first version comes from, when its module says.
fn served_key_imports(rev: &Revision) -> Result<Vec<(KeyName, Option<KeyImport>)>> {
    let catalog = super::env_store::catalog_for(rev.modules())?;
    let plan = rev
        .manifest()
        .plan(&catalog)
        .map_err(super::env::map_plan_err)?;
    Ok(plan
        .egress
        .iter()
        .flat_map(|e| e.keys())
        .map(|(module, key)| {
            (
                KeyName {
                    module: module.clone(),
                    key: key.name.clone(),
                },
                key.import.clone(),
            )
        })
        .collect())
}

/// Before `trestle up` starts `rev` of `target`'s environment: each key a module seals under in
/// it must have a keyring, or the module's server would start on none (or on a fresh one that
/// can't open what it sealed before).
///
/// - On a team's state, a revision that seals under a key is refused: no keyring can be written
///   there.
/// - A key with no keyring whose module names an import (`Provides::key_imports`) is imported,
///   once, from that variable when it resolves (the environment's vars, the workspace's, else
///   this process's environment), under the module's alias, as `trestle key import` does.
/// - Any other key with no keyring stops the start, saying what to run.
pub(super) fn ensure_keyrings(ctx: &Ctx, target: &Target, rev: &Revision) -> Result<()> {
    let keys = served_key_imports(rev)?;
    if keys.is_empty() {
        return Ok(());
    }
    let env_target = Target {
        rev: None,
        ..target.clone()
    };
    if ctx.is_shared() {
        let names: Vec<String> = keys.iter().map(|(k, _)| format!("`{k}`")).collect();
        return Err(Error::other(format!(
            "r{} of {env_target} seals under {}, whose keyring a team's state can't hold yet: \
             keyrings live in one machine's keychain, so each machine would hold a different \
             one for the same sealed data",
            rev.number(),
            names.join(", ")
        ))
        .with_hint(
            "run it on this machine's own state; a shared keyring source is not supported yet \
             (https://github.com/open-lakehouse/trestle/issues/471)",
        ));
    }
    let (_, env) = env_target.environment(&ctx.store)?;
    for (key, import) in keys {
        let var = keyring_var(&key.module, &key.key);
        if load(ctx, &env, &var)?.is_some() {
            continue;
        }
        let rotate = format!("trestle key rotate {env_target} {key}");
        let Some(import) = import else {
            return Err(Error::other(format!(
                "{key} has no keyring on {env_target}, so its module can't seal or open anything"
            ))
            .with_hint(format!("`{rotate}` makes one")));
        };
        let alias = import
            .alias
            .as_ref()
            .map(|a| format!(" --alias {a}"))
            .unwrap_or_default();
        let import_cmd = format!(
            "trestle key import {env_target} {key} --from {}{alias}",
            import.from
        );
        let Some(material) = lookup_secret(ctx, &env_target, &env, &import.from)? else {
            return Err(Error::other(format!(
                "{key} has no keyring on {env_target}, and `{}`, which {} sealed under before, \
                 is set neither on it nor in this shell: starting on a new keyring would leave \
                 what it sealed unreadable",
                import.from, key.module
            ))
            .with_hint(format!(
                "if it sealed under `{0}` before, set `{0}` again and run `trestle up` (or \
                 `{import_cmd}`); if the environment is new, `{rotate}`",
                import.from
            )));
        };
        let ring = Keyring::import(material.expose(), import.alias.as_deref())
            .map_err(|e| Error::other(format!("importing `{}` for {key}: {e}", import.from)))?;
        let resolved = Resolved {
            env_target: env_target.clone(),
            env: env.clone(),
            var,
            running: Vec::new(),
        };
        write(ctx, &resolved, None, &ring)?;
        println!(
            "imported `{}` as {}{} for {key} on {env_target}, once: from now on its keyring is \
             used, and `{}` is no longer read",
            import.from,
            kek_id(&key.key, 1),
            import
                .alias
                .as_ref()
                .map(|a| format!(" (alias {a})"))
                .unwrap_or_default(),
            import.from
        );
        println!("{BACKUP_WARNING}");
    }
    Ok(())
}

/// Writes go to this machine's keychain, which a team's other machines don't share.
fn refuse_shared(ctx: &Ctx) -> Result<()> {
    if ctx.is_shared() {
        return Err(Error::other(
            "keyrings live in one machine's keychain: on a team's state, each machine would \
             hold a different keyring for the same sealed data",
        )
        .with_hint(
            "use an environment on this machine's own state; a shared keyring source is not \
             supported yet",
        ));
    }
    Ok(())
}

/// The keyring stored for `var` on the environment, if any.
fn load(ctx: &Ctx, env: &Environment, var: &str) -> Result<Option<Keyring>> {
    match env.vars.get(var) {
        None => Ok(None),
        Some(VarValue::Secret(r)) if r.provider == KEYCHAIN => {
            let value = ctx.secrets().get(&r.key).map_err(|e| {
                Error::other(format!("reading the keyring {var}: {e}")).with_hint(
                    "the keychain entry is gone; restore it from a backup before writing a new \
                     keyring, or the data sealed under the old one stays unreadable",
                )
            })?;
            Keyring::from_json(value.expose())
                .map(Some)
                .map_err(|e| Error::other(format!("the stored keyring {var}: {e}")))
        }
        Some(_) => Err(Error::other(format!(
            "`{var}` on the environment is not a keychain secret, so it isn't a keyring \
             `trestle key` wrote"
        ))
        .with_hint(
            "a keyring variable is `trestle key`'s alone; this one was set some other way \
             (by hand, or before `trestle key`), so check what it holds before using the key",
        )),
    }
}

/// `stored` has every version from 1 up, as every keyring `trestle key` writes does. One that
/// doesn't lost a version outside `trestle key` (an edited keychain entry): building on it
/// would make the loss permanent, so it is refused until the entry is restored.
fn check_complete(stored: &Keyring, var: &str) -> Result<()> {
    let versions = stored.summary().versions;
    let top = versions.last().copied().unwrap_or(0);
    if let Some(missing) = (1..=top).find(|v| !versions.contains(v)) {
        return Err(Error::other(format!(
            "the stored keyring {var} is missing version {missing}: it was changed outside \
             `trestle key`, and anything sealed under that version can't be opened"
        ))
        .with_hint("restore the keychain entry from a backup; `trestle key` won't build on it"));
    }
    Ok(())
}

/// Whether `new` may replace the keyring: it extends `stored` (what the command read) and
/// `current` (what is stored now, should it have changed meanwhile).
fn check_write(stored: Option<&Keyring>, current: Option<&Keyring>, new: &Keyring) -> Result<()> {
    if let Some(stored) = stored {
        new.check_extends(stored)
            .map_err(|e| Error::other(e.to_string()))?;
    }
    if current != stored {
        let Some(current) = current else {
            return Err(Error::other(
                "the keyring was removed while writing; run the command again",
            ));
        };
        new.check_extends(current)
            .map_err(|e| Error::other(format!("the keyring changed while writing: {e}")))?;
    }
    Ok(())
}

/// Store `new` in place of `stored` (add-only), read it back, and check it.
fn write(ctx: &Ctx, resolved: &Resolved, stored: Option<&Keyring>, new: &Keyring) -> Result<()> {
    if let Some(stored) = stored {
        check_complete(stored, &resolved.var)?;
    }
    let (_, env) = resolved.env_target.environment(&ctx.store)?;
    let current = load(ctx, &env, &resolved.var)?;
    check_write(stored, current.as_ref(), new)?;
    let (scope, prefix, _) =
        super::var::secret_scope(ctx, &resolved.env_target, &resolved.var, false, false)?;
    super::var::store_secret(
        ctx,
        &resolved.env_target,
        (&scope, &prefix),
        &resolved.var,
        &new.to_secret(),
    )?;
    let (_, env) = resolved.env_target.environment(&ctx.store)?;
    match load(ctx, &env, &resolved.var)? {
        Some(back) if &back == new => Ok(()),
        _ => Err(Error::other(format!(
            "the keyring {} read back differs from what was written",
            resolved.var
        ))
        .with_hint("check the keychain entry before writing again")),
    }
}

/// Recreate `secret-server` in the running instances, or say the next `up` uses the keyring.
fn apply(ctx: &Ctx, resolved: &Resolved, target: &Target) -> Result<()> {
    if resolved.running.is_empty() {
        println!("no instance of {target} is running; the next `trestle up` uses it");
        return Ok(());
    }
    recreate_secret_server(ctx, &resolved.running)
}

const BACKUP_WARNING: &str = "note: the keychain holds the only copy of this keyring; without \
     it, nothing sealed under it can be read";

fn run_import(ctx: &Ctx, args: ImportArgs) -> Result<()> {
    refuse_shared(ctx)?;
    let resolved = resolve(ctx, &args.key)?;
    if load(ctx, &resolved.env, &resolved.var)?.is_some() {
        return Err(Error::other(format!(
            "{} already has a keyring on {}",
            args.key.key, resolved.env_target
        ))
        .with_hint(format!(
            "`trestle key rotate {} {}` adds a version",
            args.key.target, args.key.key
        )));
    }
    let material = match &args.from {
        Some(var) => existing_secret(ctx, &resolved, var)?,
        None => read_key(&args.key.key)?,
    };
    let ring = Keyring::import(material.expose(), args.alias.as_deref())
        .map_err(|e| Error::other(e.to_string()))?;
    write(ctx, &resolved, None, &ring)?;
    println!(
        "imported {} as {}{} on {}",
        args.key.key,
        kek_id(&args.key.key.key, 1),
        args.alias
            .as_ref()
            .map(|a| format!(" (alias {a})"))
            .unwrap_or_default(),
        resolved.env_target
    );
    println!("{BACKUP_WARNING}");
    apply(ctx, &resolved, &args.key.target)
}

/// The key to import, from stdin (or asked for).
fn read_key(key: &KeyName) -> Result<SecretValue> {
    if std::io::stdin().is_terminal() {
        let value = cliclack::password(format!("Key for {key} (32 bytes, base64)"))
            .mask('•')
            .interact()
            .map_err(Error::prompt)?;
        return Ok(SecretValue::new(value));
    }
    super::var::read_secret(&key.to_string())
}

/// The value of the existing secret `var`: the environment's, the workspace's, else this
/// process's environment.
fn existing_secret(ctx: &Ctx, resolved: &Resolved, var: &str) -> Result<SecretValue> {
    lookup_secret(ctx, &resolved.env_target, &resolved.env, var)?.ok_or_else(|| {
        Error::other(format!(
            "`{var}` is set neither on {} nor in this shell",
            resolved.env_target
        ))
    })
}

/// [`existing_secret`], `None` when `var` is set nowhere. A var set to a secret that can't be
/// read (a keychain entry gone) is an error, not `None`.
fn lookup_secret(
    ctx: &Ctx,
    env_target: &Target,
    env: &Environment,
    var: &str,
) -> Result<Option<SecretValue>> {
    super::var::check_key(var)?;
    let ws = env_target.workspace(&ctx.store)?;
    let providers = SecretProviders::new()
        .with(EnvProvider)
        .with(ctx.secrets().clone());
    let value = env.vars.get(var).or_else(|| ws.vars.get(var)).cloned();
    match value {
        Some(VarValue::Secret(r)) => providers
            .resolve(&r)
            .map(Some)
            .map_err(|e| Error::other(format!("reading {var}: {e}"))),
        Some(VarValue::Literal(v)) => Ok(Some(SecretValue::new(v))),
        None => Ok(EnvProvider.get(var).ok()),
    }
}

fn run_rotate(ctx: &Ctx, args: KeyArgs) -> Result<()> {
    refuse_shared(ctx)?;
    let resolved = resolve(ctx, &args)?;
    let stored = load(ctx, &resolved.env, &resolved.var)?;
    let ring = match &stored {
        Some(stored) => {
            let mut ring = stored.clone();
            ring.rotate().map_err(|e| Error::other(e.to_string()))?;
            ring
        }
        None => Keyring::generate().map_err(|e| Error::other(e.to_string()))?,
    };
    write(ctx, &resolved, stored.as_ref(), &ring)?;
    println!(
        "{} is active on {}",
        kek_id(&args.key.key, ring.active()),
        resolved.env_target
    );
    if ring.active() > 1 {
        println!(
            "older versions still open; once the consumer's sweep has moved them, \
             `trestle key retire {} {} --below {} --confirm-swept`",
            args.target,
            args.key,
            ring.active()
        );
    }
    println!("{BACKUP_WARNING}");
    apply(ctx, &resolved, &args.target)
}

fn run_retire(ctx: &Ctx, args: RetireArgs) -> Result<()> {
    refuse_shared(ctx)?;
    let resolved = resolve(ctx, &args.key)?;
    let Some(stored) = load(ctx, &resolved.env, &resolved.var)? else {
        return Err(Error::other(format!(
            "{} has no keyring on {}",
            args.key.key, resolved.env_target
        )));
    };
    check_complete(&stored, &resolved.var)?;
    let mut ring = stored.clone();
    let changed = ring
        .retire_below(args.below)
        .map_err(|e| Error::other(e.to_string()))?;
    if !changed {
        println!(
            "{} already unwraps only versions {} and up",
            args.key.key,
            stored.min_decrypt()
        );
        return Ok(());
    }
    if !args.confirm_swept {
        return Err(Error::other(format!(
            "retiring versions below {} makes anything still sealed under them unreadable",
            args.below
        ))
        .with_hint(format!(
            "the consumer's sweep must report zero blobs below {}: run its rotation sweep \
             (olai-store's `rewrap_all`), check `kek_usage` counts none under {}, then pass \
             `--confirm-swept`",
            args.below,
            (stored.min_decrypt()..args.below)
                .map(|v| kek_id(&args.key.key.key, v))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    write(ctx, &resolved, Some(&stored), &ring)?;
    println!(
        "{} unwraps only versions {} and up on {} (the material of the others is kept)",
        args.key.key,
        ring.min_decrypt(),
        resolved.env_target
    );
    apply(ctx, &resolved, &args.key.target)
}

/// A keyring as `key show` prints it.
#[derive(Serialize)]
struct KeyringView {
    key: String,
    variable: String,
    #[serde(flatten)]
    summary: KeyringSummary,
}

fn run_show(ctx: &Ctx, args: ShowArgs) -> Result<()> {
    let resolved = resolve(ctx, &args.key)?;
    let Some(ring) = load(ctx, &resolved.env, &resolved.var)? else {
        return Err(Error::other(format!(
            "{} has no keyring on {}",
            args.key.key, resolved.env_target
        ))
        .with_hint(format!(
            "`trestle key import {0} {1}` from an existing key, or `trestle key rotate {0} {1}` \
             for a new one",
            args.key.target, args.key.key
        )));
    };
    let view = KeyringView {
        key: args.key.key.to_string(),
        variable: resolved.var.clone(),
        summary: ring.summary(),
    };
    emit(args.json, "keyring", &view, |v| {
        println!("{} ({})", v.key, v.variable);
        let versions: Vec<String> = v
            .summary
            .versions
            .iter()
            .map(|n| {
                if *n < v.summary.min_decrypt {
                    format!("{n} (retired)")
                } else {
                    n.to_string()
                }
            })
            .collect();
        println!("  versions:    {}", versions.join(", "));
        println!("  active:      {}", v.summary.active);
        println!("  min_decrypt: {}", v.summary.min_decrypt);
        if !v.summary.aliases.is_empty() {
            let aliases: Vec<String> = v
                .summary
                .aliases
                .iter()
                .map(|(a, n)| format!("{a} → {n}"))
                .collect();
            println!("  aliases:     {}", aliases.join(", "));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
    const K2: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";

    fn ring(json: &str) -> Keyring {
        Keyring::from_json(json).unwrap()
    }

    #[test]
    fn a_write_must_extend_what_was_read_and_what_is_stored() {
        let stored = Keyring::import(K1, Some("kek-1")).unwrap();
        let mut rotated = stored.clone();
        rotated.rotate().unwrap();
        check_write(Some(&stored), Some(&stored), &rotated).unwrap();
        check_write(None, None, &stored).unwrap();

        // A write that drops version 1.
        let dropped = ring(&format!(r#"{{"versions": {{"2": "{K2}"}}, "active": 2}}"#));
        let err = check_write(Some(&stored), Some(&stored), &dropped).unwrap_err();
        assert!(err.to_string().contains("drop version 1"), "{err}");

        // Changed underneath: version 1 swapped, or the keyring gone, or one imported.
        let swapped = Keyring::import(K2, Some("kek-1")).unwrap();
        let err = check_write(Some(&stored), Some(&swapped), &rotated).unwrap_err();
        assert!(err.to_string().contains("changed while writing"), "{err}");
        let err = check_write(Some(&stored), None, &rotated).unwrap_err();
        assert!(err.to_string().contains("removed while writing"), "{err}");
        let err = check_write(None, Some(&stored), &swapped).unwrap_err();
        assert!(err.to_string().contains("changed while writing"), "{err}");
    }

    #[test]
    fn a_keyring_that_lost_a_version_is_not_built_on() {
        let mut complete = Keyring::import(K1, None).unwrap();
        complete.rotate().unwrap();
        check_complete(&complete, "V").unwrap();
        let gapped = ring(&format!(
            r#"{{"versions": {{"1": "{K1}", "3": "{K2}"}}, "active": 3}}"#
        ));
        let err = check_complete(&gapped, "V").unwrap_err();
        assert!(err.to_string().contains("missing version 2"), "{err}");
    }

    #[test]
    fn key_names_are_module_slash_key() {
        let name: KeyName = "keys-app/server-kek".parse().unwrap();
        assert_eq!(name.module.as_str(), "keys-app");
        assert_eq!(name.key, "server-kek");
        assert_eq!(name.to_string(), "keys-app/server-kek");
        for bad in ["server-kek", "/k", "m/", "a/b/c"] {
            assert!(bad.parse::<KeyName>().is_err(), "{bad}");
        }
    }
}
