//! `trestle var`: the variables a workspace, an environment or a revision carries.
//!
//! The target picks the set: `<ws>` is the workspace's (every environment sees it), `<ws>/<env>`
//! the environment's (every revision sees it), and `<ws>/<env> --draft` the draft's (frozen into
//! the next revision by `env commit`). `var list <ws>/<env>@<n>` reads a revision's.
//!
//! A `--secret` value goes to the OS keychain; the store keeps only a reference to it.

use std::fmt;
use std::io::{IsTerminal, Read};

use clap::{Args, Subcommand};
use olai_stack_runtime::SecretValue;
use olai_stack_state::{SecretRef, VarScope, VarSet, VarValue};
use serde::Serialize;

use super::output::emit;
use super::state::{Ctx, KEYCHAIN, environment_keychain_refs};
use super::target::{Rev, Target, revision};
use crate::error::{Error, Result};

/// The `trestle var` subcommand group.
#[derive(Subcommand)]
pub enum VarCommand {
    /// Set variables: `KEY=VALUE`, or `KEY` with `--secret` (the value is read from stdin or
    /// asked for).
    Set(SetArgs),
    /// Remove variables (and their keychain entries).
    Unset(UnsetArgs),
    /// List variables.
    List(ListArgs),
}

#[derive(Args, Clone)]
pub struct SetArgs {
    /// `<ws>` or `<ws>/<env>`.
    pub target: Target,

    /// `KEY=VALUE` (or `KEY` with `--secret`).
    #[clap(required = true)]
    pub vars: Vec<String>,

    /// Store the value in the OS keychain, read from stdin (never from the command line).
    #[clap(long)]
    pub secret: bool,

    /// Set it on the draft, so the next revision carries it, rather than on the environment.
    #[clap(long)]
    pub draft: bool,
}

#[derive(Args, Clone)]
pub struct UnsetArgs {
    /// `<ws>` or `<ws>/<env>`.
    pub target: Target,

    /// The variable names.
    #[clap(required = true)]
    pub keys: Vec<String>,

    /// Remove it from the draft rather than from the environment.
    #[clap(long)]
    pub draft: bool,
}

#[derive(Args, Clone)]
pub struct ListArgs {
    /// `<ws>`, `<ws>/<env>` (with the draft's), `<ws>/<env>@<n>` or `<ws>/<env>@draft`.
    pub target: Target,

    /// Print as JSON.
    #[clap(long)]
    pub json: bool,
}

pub fn run(cmd: VarCommand) -> Result<()> {
    let ctx = Ctx::open()?;
    match cmd {
        VarCommand::Set(args) => run_set(&ctx, args),
        VarCommand::Unset(args) => run_unset(&ctx, args),
        VarCommand::List(args) => run_list(&ctx, args),
    }
}

/// The var set `target` (with `--draft`) names, and the keychain key prefix of its secrets.
///
/// The prefix is built from ids, not names: a workspace or environment can be renamed, and a
/// new one may then take the old name. Name-based keys would let its secrets overwrite the
/// keychain entries the renamed one still refers to.
fn scope(ctx: &Ctx, target: &Target, draft: bool) -> Result<(VarScope, String)> {
    target.no_rev()?;
    match &target.env {
        None if draft => Err(Error::other("--draft needs an environment (<ws>/<env>)")),
        None => {
            let ws = target.workspace(&ctx.store)?;
            let prefix = ws.id.0.clone();
            Ok((VarScope::Workspace(ws.id), prefix))
        }
        Some(_) => {
            let (_, env) = target.environment(&ctx.store)?;
            if draft {
                // Each revision's secrets get keys of their own, so a later edit of the draft
                // never changes what a committed revision resolves to.
                let next = env.head.unwrap_or(0) + 1;
                let prefix = format!("{}@r{next}", env.id);
                Ok((VarScope::Draft(env.id), prefix))
            } else {
                let prefix = env.id.0.clone();
                Ok((VarScope::Environment(env.id), prefix))
            }
        }
    }
}

fn check_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let valid = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(Error::other(format!(
            "`{key}` is not a variable name (letters, digits and `_`, not starting with a digit)"
        )))
    }
}

fn run_set(ctx: &Ctx, args: SetArgs) -> Result<()> {
    let (scope, prefix) = scope(ctx, &args.target, args.draft)?;
    let mut changes = Vec::new();
    for var in &args.vars {
        if args.secret {
            if var.contains('=') {
                return Err(Error::other(
                    "a secret's value is read from stdin, not given on the command line",
                )
                .with_hint(format!(
                    "`printf %s \"$VALUE\" | trestle var set {} --secret {}`",
                    args.target,
                    var.split('=').next().unwrap_or_default()
                )));
            }
            check_key(var)?;
            let value = read_secret(var)?;
            let key = format!("{prefix}/{var}");
            ctx.secrets().set(&key, &value)?;
            changes.push((
                var.clone(),
                VarValue::Secret(SecretRef {
                    provider: KEYCHAIN.into(),
                    key,
                }),
            ));
        } else {
            let (key, value) = var.split_once('=').ok_or_else(|| {
                Error::other(format!("expected KEY=VALUE, got `{var}`"))
                    .with_hint("pass `--secret KEY` to read a secret from stdin")
            })?;
            check_key(key)?;
            changes.push((key.to_owned(), VarValue::Literal(value.to_owned())));
        }
    }
    let mut replaced = Vec::new();
    ctx.store.modify_vars(&scope, |vars| {
        for (key, value) in changes {
            if let Some(old) = vars.0.insert(key, value) {
                replaced.push(old);
            }
        }
        Ok::<_, Error>(())
    })?;
    // A literal that replaced a secret leaves its keychain entry unused.
    forget_unused(ctx, &args.target, replaced)?;
    println!(
        "set {} on {}",
        args.vars.len(),
        describe(&args.target, args.draft)
    );
    Ok(())
}

fn run_unset(ctx: &Ctx, args: UnsetArgs) -> Result<()> {
    let (scope, _) = scope(ctx, &args.target, args.draft)?;
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    ctx.store.modify_vars(&scope, |vars| {
        for key in &args.keys {
            match vars.remove(key) {
                Some(v) => removed.push(v),
                None => missing.push(key.clone()),
            }
        }
        Ok::<_, Error>(())
    })?;
    let count = removed.len();
    forget_unused(ctx, &args.target, removed)?;
    for key in missing {
        eprintln!("warning: `{key}` was not set");
    }
    println!("unset {count} on {}", describe(&args.target, args.draft));
    Ok(())
}

/// Delete the keychain entries of `dropped` secrets nothing in the target's workspace still
/// refers to (a committed revision may).
fn forget_unused(ctx: &Ctx, target: &Target, dropped: Vec<VarValue>) -> Result<()> {
    let refs: Vec<SecretRef> = dropped
        .into_iter()
        .filter_map(|v| match v {
            VarValue::Secret(r) if r.provider == KEYCHAIN => Some(r),
            _ => None,
        })
        .collect();
    if refs.is_empty() {
        return Ok(());
    }
    let ws = target.workspace(&ctx.store)?;
    let mut used: Vec<SecretRef> = super::state::keychain_refs(&ws.vars).cloned().collect();
    for env in ctx.store.environments(&ws.id)? {
        used.extend(environment_keychain_refs(&ctx.store, &env)?);
    }
    for r in refs {
        if !used.contains(&r) {
            ctx.secrets().delete(&r.key)?;
        }
    }
    Ok(())
}

fn describe(target: &Target, draft: bool) -> String {
    if draft {
        format!("{target} (draft)")
    } else {
        target.to_string()
    }
}

/// A secret's value: stdin when it isn't a terminal, else a prompt.
fn read_secret(key: &str) -> Result<SecretValue> {
    let value = if std::io::stdin().is_terminal() {
        cliclack::password(format!("Value of {key}"))
            .mask('•')
            .interact()
            .map_err(Error::prompt)?
    } else {
        let mut value = String::new();
        std::io::stdin().read_to_string(&mut value)?;
        // `echo` adds a newline the value doesn't mean to have.
        value.trim_end_matches(['\r', '\n']).to_owned()
    };
    if value.is_empty() {
        return Err(Error::other(format!("no value given for `{key}`")));
    }
    Ok(SecretValue::new(value))
}

/// A variable as `var list` and `ws show` print it. A secret shows where it lives, never its
/// value.
#[derive(Serialize)]
pub(super) struct VarRow {
    pub name: String,
    /// `workspace`, `environment`, `draft` or `revision`.
    pub scope: &'static str,
    /// The literal value; `None` for a secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Where a secret lives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretRef>,
}

impl fmt::Display for VarRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.value, &self.secret) {
            (Some(v), _) => write!(f, "{}={v}", self.name),
            (None, Some(s)) => write!(f, "{}=<secret {}:{}>", self.name, s.provider, s.key),
            (None, None) => write!(f, "{}", self.name),
        }
    }
}

/// `vars` as rows of `scope`.
pub(super) fn var_rows(vars: &VarSet, scope: &'static str) -> Vec<VarRow> {
    vars.0
        .iter()
        .map(|(name, v)| {
            let (value, secret) = match v {
                VarValue::Literal(l) => (Some(l.clone()), None),
                VarValue::Secret(r) => (None, Some(r.clone())),
            };
            VarRow {
                name: name.clone(),
                scope,
                value,
                secret,
            }
        })
        .collect()
}

fn run_list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    let rows = match &args.target.env {
        None => var_rows(&args.target.workspace(&ctx.store)?.vars, "workspace"),
        Some(_) => {
            let (_, env) = args.target.environment(&ctx.store)?;
            match args.target.rev {
                None => {
                    let mut rows = var_rows(&env.vars, "environment");
                    rows.extend(var_rows(&env.draft.vars, "draft"));
                    rows
                }
                Some(Rev::Draft) => var_rows(&env.draft.vars, "draft"),
                rev => var_rows(revision(&ctx.store, &env, rev)?.vars(), "revision"),
            }
        }
    };
    emit(args.json, "vars", &rows, |rows| {
        if rows.is_empty() {
            println!("no vars on {}", args.target);
        }
        for r in rows {
            println!("{:<12} {r}", r.scope);
        }
    })
}
