//! The **Docker Compose** rendering of a [`Plan`]: every module's compose fragment and mounted
//! files, the top-level `compose.yaml` that `include:`s them, the `.env` substitution overlay,
//! and the on-disk layout they are written in ([`materialize`]).
//!
//! The plan itself is target-neutral — it carries each module's
//! [`ModuleBinding`](crate::ModuleBinding) and injected env, never rendered text. This module is
//! where those become Compose: each module's [`RenderSpec`](crate::RenderSpec) renders against
//! its binding, the files it emits are rooted under `modules/<id>/`, and their `configs:` /
//! `secrets:` aliases are aggregated into the [`HeadFile`].
//!
//! Everything here is pure and string-only — no I/O. The top-level shapes are hand-written (not
//! serialized via a YAML library) to stay reviewable and stable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::catalog::module::ModuleId;
use crate::model::placement::Vantage;
use crate::plan::Plan;
use crate::render::envoy::render_envoy;
use crate::render::output::{
    CLIENT_CONTAINER_ENV_PATH, CLIENT_HOST_ENV_PATH, MaterializedOutput, OutputFile,
    render_client_env,
};
use crate::render::{InjectedEnv, RenderError, RenderOutput};
use crate::target::TargetId;

/// A compose `include:` entry contributed by a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeInclude {
    /// The module that contributed this fragment.
    pub module: ModuleId,
    /// The fragment text (the module's rendered compose `services:` snippet).
    pub fragment: String,
}

/// A top-level compose `configs:` declaration: an alias and the host file it maps to.
///
/// The generated compose emits `configs: <alias>: { file: <path> }`, and a service
/// fragment mounts it by `configs: - source: <alias>`. Each entry comes from a module's
/// [`RenderFile`](crate::RenderFile) that declared an `alias` (the host `path` is the
/// per-module-rooted file path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDecl {
    /// The compose config alias (referenced by `configs: - source: <alias>`).
    pub alias: String,
    /// The host file path the alias maps to (e.g. `modules/envoy/envoy.yaml`).
    pub path: String,
}

/// A top-level Compose `secrets:` declaration: an alias and its sensitive host file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretDecl {
    /// The alias mounted by services under `/run/secrets/<alias>`.
    pub alias: String,
    /// The host file path supplying the secret.
    pub path: String,
}

/// The consolidated top-level compose plan: customization at the head (injected env),
/// then a plain list of module fragments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadFile {
    /// The compose project name.
    pub name: String,
    /// The environment-wide injected variables, gathered at the head.
    pub env: InjectedEnv,
    /// The module fragments to include, in dependency order.
    pub includes: Vec<ComposeInclude>,
    /// The top-level `configs:` declarations (alias → host file), sorted by alias for
    /// deterministic output. Aggregated from every module's aliased
    /// [`RenderFile`](crate::RenderFile)s plus any synthetic entry the planner adds for a
    /// dedicated-renderer artifact (e.g. the gateway's envoy bootstrap).
    pub configs: Vec<ConfigDecl>,
    /// Top-level Compose secret declarations, sorted by alias.
    pub secrets: Vec<SecretDecl>,
}

/// Render every module of `plan` in dependency order and assemble the [`HeadFile`].
///
/// A module with an empty fragment (e.g. the env-only contract module) contributes no compose
/// include. Each emitted file is rooted under `modules/<id>/` (unless marked `at_root`), and its
/// `configs:` / `secrets:` alias is declared at the head — aliases share one namespace, so a
/// second module reusing one is a [`RenderError::AliasCollision`].
pub fn render_modules(
    plan: &Plan,
) -> Result<(Vec<(ModuleId, RenderOutput)>, HeadFile), RenderError> {
    let mut renders = Vec::with_capacity(plan.graph.nodes.len());
    let mut includes = Vec::new();
    let mut configs: Vec<ConfigDecl> = Vec::new();
    let mut secrets: Vec<SecretDecl> = Vec::new();
    // The module each alias was first seen on, so a second module reusing an alias is rejected
    // rather than silently shadowing.
    let mut alias_owner: BTreeMap<String, ModuleId> = BTreeMap::new();
    // An external provider runs outside the stack: nothing to render for it.
    for module in plan
        .graph
        .nodes
        .iter()
        .filter(|m| !plan.external.contains_key(m.id()))
    {
        let id = module.id();
        let render_ctx = plan.bindings[id].render_ctx(&plan.injected[id]);
        let spec = module.render_for(&TargetId::compose()).ok_or_else(|| {
            RenderError::UnsupportedOnTarget {
                module: id.clone(),
                target: TargetId::compose(),
            }
        })?;
        let mut out = spec
            .render(&render_ctx)
            .map_err(|source| RenderError::Template {
                module: id.clone(),
                source,
            })?;
        // Root each emitted file under the module's own directory (`modules/<id>/<path>`),
        // unless the module marked it `at_root` (operator-facing files live beside
        // `compose.yaml`). The rewritten path is the single source of truth the consumer
        // writes to and the compose `configs: file:` / `secrets: file:` references.
        for f in &mut out.files {
            if !f.at_root {
                f.path = format!("modules/{}/{}", id.as_str(), f.path);
            }
            // A file an external resource's secret was resolved into holds a credential.
            f.sensitive |= plan.secret_values.appear_in(&f.contents);
            if f.alias.is_some() && f.secret_alias.is_some() {
                return Err(RenderError::ConfigAndSecret {
                    module: id.clone(),
                    path: f.path.clone(),
                });
            }
            if let Some(alias) = &f.alias {
                claim_alias(&mut alias_owner, alias, id)?;
                configs.push(ConfigDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
            if let Some(alias) = &f.secret_alias {
                claim_alias(&mut alias_owner, alias, id)?;
                secrets.push(SecretDecl {
                    alias: alias.clone(),
                    path: f.path.clone(),
                });
            }
        }
        if !out.fragment.trim().is_empty() {
            includes.push(ComposeInclude {
                module: id.clone(),
                fragment: out.fragment.clone(),
            });
        }
        renders.push((id.clone(), out));
    }

    // Deterministic `configs:` / `secrets:` order regardless of module/graph order.
    configs.sort_by(|a, b| a.alias.cmp(&b.alias));
    secrets.sort_by(|a, b| a.alias.cmp(&b.alias));

    let head = HeadFile {
        name: plan.env_name.clone(),
        env: plan.env.clone(),
        includes,
        configs,
        secrets,
    };
    Ok((renders, head))
}

/// Record `module` as the owner of the top-level `alias`, rejecting a second owner.
fn claim_alias(
    owner: &mut BTreeMap<String, ModuleId>,
    alias: &str,
    module: &ModuleId,
) -> Result<(), RenderError> {
    if let Some(first) = owner.get(alias) {
        return Err(RenderError::AliasCollision {
            alias: alias.to_string(),
            first: first.clone(),
            second: module.clone(),
        });
    }
    owner.insert(alias.to_string(), module.clone());
    Ok(())
}

/// Append each line followed by `\n`. Used by the hand-built (non-templated) artifact
/// renderers below — the Postgres init script, the `.env` overlay, and the top-level
/// compose file, whose shapes are small, line-oriented, and not worth a template.
fn push_lines(out: &mut String, lines: &[&str]) {
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
}

/// Collect the compose variable names referenced as `${NAME}` (or `${NAME:-default}` /
/// `${NAME:?err}`) across `texts`. Only brace substitutions count — a brace-less shell `$VAR`
/// (e.g. `$POSTGRES_USER` inside an init heredoc) is a container-runtime read, not a compose
/// substitution the `.env` overlay must supply, so it is deliberately ignored.
fn referenced_env_vars<'a>(texts: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for text in texts {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let mut j = i + 2;
                // A variable name: leading non-digit identifier char, then identifier chars.
                let start = j;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let name = &text[start..j];
                // Valid only if non-empty, not starting with a digit, and the braces close
                // (possibly after a `:-default` / `:?err` modifier).
                if !name.is_empty()
                    && !name.as_bytes()[0].is_ascii_digit()
                    && text[j..].find('}').is_some()
                {
                    refs.insert(name.to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }
    refs
}

/// Render the `.env` overlay: `KEY=value` lines in deterministic (key) order, emitting only
/// keys some rendered artifact references as `${KEY}` (see [`referenced_env_vars`]). Keys the
/// templates consumed concretely at plan time (e.g. `BASE_PATH`, `DATA_ROOT`) are referenced
/// by nothing and so are naturally omitted.
fn render_env(env: &InjectedEnv, referenced: &BTreeSet<String>) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "# Compose substitution overlay: only variables some rendered file references as",
            "# `${VAR}`. Clients (apps, notebooks, engines) load `client/host.env` or",
            "# `client/container.env` instead.",
        ],
    );
    for (k, v) in env.iter() {
        if referenced.contains(k) {
            let _ = writeln!(out, "{k}={v}");
        }
    }
    out
}

/// Render the top-level `compose.yaml`: the project name, a top-level `configs:` block
/// declaring every module-announced config file (alias → host file), and an `include:` per
/// module fragment, in dependency order.
///
/// Each module lives in its own directory: its fragment is `./modules/<id>/compose.yaml` and
/// its config files sit beside it (`./modules/<id>/<file>`). `project_directory: .` keeps a
/// fragment's relative references resolving against this top-level file, so the `configs:`
/// `file:` paths below line up with what the consumer writes.
pub fn render_compose(head: &HeadFile) -> String {
    let mut out = String::new();
    push_lines(
        &mut out,
        &[
            "# Top-level compose entrypoint. Every backing service is pulled in via `include:`.",
            "# The file list is owned by the planner, not edited by hand.",
            &format!("name: {}", head.name),
        ],
    );
    if !head.configs.is_empty() {
        push_lines(&mut out, &["", "configs:"]);
        for c in &head.configs {
            push_lines(
                &mut out,
                &[
                    &format!("  {}:", c.alias),
                    &format!("    file: ./{}", c.path),
                ],
            );
        }
    }
    if !head.secrets.is_empty() {
        push_lines(&mut out, &["", "secrets:"]);
        for secret in &head.secrets {
            push_lines(
                &mut out,
                &[
                    &format!("  {}:", secret.alias),
                    &format!("    file: ./{}", secret.path),
                ],
            );
        }
    }
    push_lines(&mut out, &["", "include:"]);
    for inc in &head.includes {
        push_lines(
            &mut out,
            &[
                // Each module's fragment is co-located in its own directory.
                &format!("  - path: ./modules/{}/compose.yaml", inc.module.as_str()),
                "    project_directory: .",
            ],
        );
    }
    out
}

/// The compose `configs:` alias the gateway's Envoy bootstrap is mounted under. The bootstrap
/// is produced by the dedicated [`render_envoy`] renderer (not a module [`RenderFile`]), so the
/// planner does not see it; [`render_all`] declares it as a synthetic config when the gateway
/// is present.
const ENVOY_CONFIG_ALIAS: &str = "envoy_config";

/// The per-module-rooted host path the Envoy bootstrap is written to.
const ENVOY_CONFIG_PATH: &str = "modules/envoy/envoy.yaml";

/// Everything the Compose rendering of a plan produces, before it is laid out on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifacts {
    /// Each module's rendered fragment and files, in dependency order (file paths already
    /// rooted under `modules/<id>/`, or at the root when marked `at_root`).
    pub modules: Vec<(ModuleId, RenderOutput)>,
    /// The consolidated head the top-level `compose.yaml` is rendered from.
    pub head: HeadFile,
    /// The Envoy bootstrap config. Written to `modules/envoy/envoy.yaml` and mounted by the
    /// gateway fragment via the `envoy_config` config alias.
    pub envoy: String,
    /// The `.env` overlay.
    pub env: String,
    /// The top-level `compose.yaml`.
    pub compose: String,
    /// Ignore rules for runtime data and generated credential-bearing files.
    pub gitignore: String,
}

/// Render `plan` for Docker Compose: every module (see [`render_modules`]), the Envoy bootstrap
/// (a dedicated renderer), the audited `.env`, and the top-level `compose.yaml`.
pub fn render_all(plan: &Plan) -> Result<Artifacts, RenderError> {
    let (modules, mut head) = render_modules(plan)?;
    let envoy = render_envoy(&plan.gateway);

    // The `.env` carries only keys some rendered artifact still references as `${KEY}`. Scan
    // every module fragment, every mounted file's contents, and the Envoy bootstrap.
    let referenced = referenced_env_vars(
        modules
            .iter()
            .flat_map(|(_, out)| {
                std::iter::once(out.fragment.as_str())
                    .chain(out.files.iter().map(|f| f.contents.as_str()))
            })
            .chain(std::iter::once(envoy.as_str())),
    );

    // The Envoy bootstrap is a dedicated-renderer artifact, so add its `configs:` declaration
    // here rather than from a module's `RenderFile`. Skip it if a module already declared this
    // alias, so the synthetic push never emits a duplicate top-level key.
    let already_declared = head.configs.iter().any(|c| c.alias == ENVOY_CONFIG_ALIAS);
    if has_gateway(plan) && !already_declared {
        head.configs.push(ConfigDecl {
            alias: ENVOY_CONFIG_ALIAS.into(),
            path: ENVOY_CONFIG_PATH.into(),
        });
        head.configs.sort_by(|a, b| a.alias.cmp(&b.alias));
    }

    Ok(Artifacts {
        env: render_env(&plan.env, &referenced),
        compose: render_compose(&head),
        gitignore: render_gitignore(plan, &modules),
        envoy,
        modules,
        head,
    })
}

/// The `.gitignore`: runtime data and the generated credential-bearing directories, plus every
/// rendered file an external resource's secret was resolved into (they can sit anywhere a
/// template writes, e.g. a mounted config file).
fn render_gitignore(plan: &Plan, modules: &[(ModuleId, RenderOutput)]) -> String {
    let mut out = String::from(".data/\n.env\nsecrets.env\nmodules/*/.env/\nmodules/*/secrets/\n");
    let mut secret_files: Vec<String> = Vec::new();
    for (id, render) in modules {
        if plan.secret_values.appear_in(&render.fragment) {
            secret_files.push(format!("modules/{id}/compose.yaml"));
        }
        secret_files.extend(
            render
                .files
                .iter()
                .filter(|f| plan.secret_values.appear_in(&f.contents))
                .map(|f| f.path.clone()),
        );
    }
    // Skip what the patterns above already cover (`modules/<id>/{.env,secrets}/…`).
    secret_files.retain(|path| {
        let mut parts = path.split('/');
        !matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some("modules"), Some(_), Some(".env" | "secrets"))
        )
    });
    if !secret_files.is_empty() {
        out.push_str("# Rendered with external-resource secrets:\n");
        for path in secret_files {
            let _ = writeln!(out, "{path}");
        }
    }
    out
}

/// Whether the plan has a gateway. Detected by the `gateway` role on a resolved service, not a
/// module-id string, so a differently-named gateway module is still covered.
fn has_gateway(plan: &Plan) -> bool {
    plan.services
        .values()
        .flatten()
        .any(|s| s.role == crate::Role::gateway())
}

/// Render `plan` for Docker Compose and flatten everything into the on-disk layout a consumer
/// writes (see [`MaterializedOutput`] for the layout).
pub fn materialize(plan: &Plan) -> Result<MaterializedOutput, RenderError> {
    let artifacts = render_all(plan)?;
    let mut files = vec![
        OutputFile {
            path: "compose.yaml".into(),
            contents: artifacts.compose,
            sensitive: false,
            preserve: false,
        },
        OutputFile {
            path: ".env".into(),
            contents: artifacts.env,
            sensitive: true,
            preserve: false,
        },
        OutputFile {
            path: ".gitignore".into(),
            contents: artifacts.gitignore,
            sensitive: false,
            preserve: false,
        },
        // What a client loads to reach the stack, per vantage. Credential-free, so not
        // sensitive: it is meant to be shared (e.g. sourced by an app's dev environment).
        OutputFile {
            path: CLIENT_HOST_ENV_PATH.into(),
            contents: render_client_env(
                plan.client_env(Vantage::Host),
                "on the host (localhost addresses)",
            ),
            sensitive: false,
            preserve: false,
        },
        OutputFile {
            path: CLIENT_CONTAINER_ENV_PATH.into(),
            contents: render_client_env(
                plan.client_env(Vantage::Container),
                "inside the compose network",
            ),
            sensitive: false,
            preserve: false,
        },
        // A human-readable, at-a-glance summary of the gateway layout (routes → services).
        // Pure to build (see `report::layout_report`), so it keeps `materialize` pure.
        OutputFile {
            path: "LAYOUT.md".into(),
            contents: crate::report::layout_report(plan),
            sensitive: false,
            preserve: false,
        },
    ];

    // The Envoy bootstrap is written only when the plan has a gateway — the same condition
    // under which `render_all` declares the `envoy_config` mount. A gateway-less plan would
    // otherwise get an orphan `modules/envoy/envoy.yaml` that the top-level compose never
    // mounts. Detect the gateway by role (not module-id), matching `render_all`.
    if has_gateway(plan) {
        files.push(OutputFile {
            path: ENVOY_CONFIG_PATH.into(),
            contents: artifacts.envoy,
            sensitive: false,
            preserve: false,
        });
    }

    // Each module owns a `modules/<id>/` directory: its compose fragment (skipped when empty)
    // plus any config files it emits (their `path` is already rooted under that directory,
    // or at the environment root when marked `at_root`).
    for (module, out) in &artifacts.modules {
        if !out.fragment.trim().is_empty() {
            files.push(OutputFile {
                path: format!("modules/{module}/compose.yaml"),
                contents: out.fragment.clone(),
                sensitive: plan.secret_values.appear_in(&out.fragment),
                preserve: false,
            });
        }
        for file in &out.files {
            files.push(OutputFile {
                path: file.path.clone(),
                contents: file.contents.clone(),
                sensitive: file.sensitive,
                preserve: file.preserve,
            });
        }
    }

    // The Python client workspace, when selected, sits beside the client environments.
    files.extend(crate::render::python_client::python_client_files(plan));

    Ok(MaterializedOutput { files })
}
