//! Phase 4 — **bind**: hand every module the values the planner decided for it.
//!
//! Per module this produces its [`InjectedEnv`] (stack facts, declared env vars, the
//! coordinates bound for its demands, its base path) and a [`ModuleBinding`] — the typed,
//! target-neutral data its templates render against (connections, readiness gates, provisioned
//! objects, the gateway's published ports and TLS material). It also aggregates the stack-wide
//! environment. Nothing here is rendered: a deploy target turns the bindings into files.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::demands::{bind_connection, provider_ready_gate};
use super::route::Routed;
use super::settle::Settled;
use super::{GatewayConfig, PlanCtx, PlanError};
use crate::catalog::Catalog;
use crate::catalog::baseline::{
    CLIENT_ENV_KEYS_VAR, DATA_ROOT_VAR, GATEWAY_AUTH_VAR, GATEWAY_HOST_VAR,
    GATEWAY_INTERNAL_HOST_VAR, GATEWAY_INTERNAL_URL_VAR, GATEWAY_URL_VAR,
};
use crate::catalog::module::{
    GatewayTls, ModuleId, PortMapping, Readiness, ReadyGate, RenderCtx, ResolvedKnobs,
};
use crate::model::connection::Connection;
use crate::model::placement::Vantage;
use crate::model::role::Role;
use crate::render::InjectedEnv;

/// Everything a module's templates render against besides its [`InjectedEnv`]: the planner's
/// typed, per-module decisions. Target-neutral — a deploy target reads it (via
/// [`render_ctx`](Self::render_ctx)) to render the module in its own format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModuleBinding {
    /// The module's resolved config knobs.
    pub knobs: ResolvedKnobs,
    /// The typed connections for the module's demands, keyed by resource role — plus, for a
    /// provider, its own role's connection resolved for each name it provisions.
    pub connections: BTreeMap<String, Vec<Connection>>,
    /// The readiness gates the module waits on before starting, in demand order.
    pub dependencies: Vec<ReadyGate>,
    /// The resource names this module provisions for its own role (empty for a non-provider).
    pub objects: Vec<String>,
    /// The gateway's `host:container` listener mappings (gateway module only).
    pub published_ports: Vec<PortMapping>,
    /// The gateway's TLS material (gateway module only, when it serves TLS).
    pub tls: Option<GatewayTls>,
}

impl ModuleBinding {
    /// The [`RenderCtx`] a module's [`RenderSpec`](crate::RenderSpec) renders against: this
    /// binding together with the module's `env`.
    pub fn render_ctx<'a>(&'a self, env: &'a InjectedEnv) -> RenderCtx<'a> {
        RenderCtx {
            env,
            knobs: &self.knobs,
            connections: self.connections.clone(),
            dependencies: self.dependencies.clone(),
            objects: self.objects.clone(),
            published_ports: self.published_ports.clone(),
            tls: self.tls.clone(),
        }
    }
}

/// The bind phase's output.
pub(super) struct Bound {
    /// Each module's injected env.
    pub injected: BTreeMap<ModuleId, InjectedEnv>,
    /// Each module's typed render data.
    pub bindings: BTreeMap<ModuleId, ModuleBinding>,
    /// The stack's aggregated environment variables, last-writer-wins in dependency order.
    pub env: InjectedEnv,
}

/// Run the bind phase (see the [module docs](self)).
pub(super) fn bind(
    settled: &Settled,
    routed: &Routed,
    catalog: &Catalog,
    ctx: &PlanCtx,
) -> Result<Bound, PlanError> {
    let Settled {
        graph,
        chosen,
        knobs,
        stack_facts,
        services: services_of,
        provisioned,
    } = settled;
    let gateway = &routed.gateway;

    // What a host-side browser reaches the shared listener at (see `GATEWAY_URL_VAR`), and
    // what an in-network caller reaches it at (see `GATEWAY_INTERNAL_URL_VAR`).
    let (gateway_url, gateway_host) = routed.facts.origin(Vantage::Host);
    let (gateway_internal_url, gateway_internal_host) = routed.facts.origin(Vantage::Container);
    // Every client-environment key the stack defines (see `CLIENT_ENV_KEYS_VAR`).
    let client_env_keys = graph
        .nodes
        .iter()
        .flat_map(|m| m.provides().client_env.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");

    let mut injected = BTreeMap::new();
    for module in &graph.nodes {
        let mut module_env = InjectedEnv::new();
        // The stack's root data directory, available to every fragment as a render-only value
        // (baked at plan time, like `BASE_PATH`). A module that persists state mounts it under
        // `{{ env.DATA_ROOT }}/<module>` rather than hard-coding a `./.data/...` path, so the
        // whole stack's persistence relocates via the one `PlanCtx::data_root` knob.
        module_env.set(DATA_ROOT_VAR, &ctx.data_root);
        module_env.set(GATEWAY_URL_VAR, &gateway_url);
        module_env.set(GATEWAY_HOST_VAR, &gateway_host);
        module_env.set(GATEWAY_INTERNAL_URL_VAR, &gateway_internal_url);
        module_env.set(GATEWAY_INTERNAL_HOST_VAR, &gateway_internal_host);
        module_env.set(
            GATEWAY_AUTH_VAR,
            if stack_facts.gateway_auth {
                "true"
            } else {
                "false"
            },
        );
        module_env.set(CLIENT_ENV_KEYS_VAR, &client_env_keys);
        // Seed each module's render env with its declared env vars.
        for (k, v) in module.provides().env_vars.iter() {
            module_env.set(k, v);
        }
        // Config knobs stay out of `env` — they are passed separately as `RenderCtx.knobs`
        // so templates read `{{ knobs.image }}` rather than conflating them with planner
        // injections / container env vars.
        // Bind each demand's resolved connection back into the consuming module's env, by
        // typed field.
        for (idx, demand) in module.needs().iter().enumerate() {
            let connection = &chosen[&(module.id().clone(), idx)].connection;
            bind_connection(&mut module_env, module.id(), demand, connection)?;
        }
        // A service that relocates its surface under a base path is told it at render time
        // (e.g. via a `[ui].base_path` config line), whichever endpoint intents it routes the
        // prefix with.
        for service in &services_of[module.id()] {
            if !service.base_path.is_empty() {
                module_env.set("BASE_PATH", &service.base_path);
            }
        }
        injected.insert(module.id().clone(), module_env);
    }

    // Aggregate the stack's env vars for `.env`, last-writer-wins in dependency order:
    // each module's *declared* env vars, plus the coordinates injected to satisfy its
    // demands (a fragment reads those as `${VAR}`, so they must resolve from `.env` at run
    // time). Render-only injections (`BASE_PATH`, `DATA_ROOT`) stay out of `.env` — they are
    // rendered into the fragment at plan time.
    let mut env = InjectedEnv::new();
    for module in &graph.nodes {
        for (k, v) in module.provides().env_vars.iter() {
            env.set(k, v);
        }
        // A provider contributes its connections' conventional SDK env vars (an object
        // store's `AWS_*` / `AZURE_STORAGE_CONNECTION_STRING`), derived from the typed
        // credential so it is stated once. Only an in-graph (i.e. chosen) provider reaches
        // here, so an unselected backend's credentials never leak into `.env`. The values
        // are name-independent, so the template's credential is read directly.
        for template in module.provides().resource_kinds.values() {
            for (k, v) in template.0.standard_env() {
                env.set(k, v);
            }
        }
        for (idx, demand) in module.needs().iter().enumerate() {
            let connection = &chosen[&(module.id().clone(), idx)].connection;
            bind_connection(&mut env, module.id(), demand, connection)?;
        }
    }

    let mut bindings = BTreeMap::new();
    for module in &graph.nodes {
        // The typed connections resolved for this module's demands, grouped by role, so a
        // template can branch on the chosen credential flavour. Alongside them, the resolved
        // readiness gates — one per demand whose chosen provider advertises a startup gate —
        // so a template iterates them instead of hard-coding which backend it waits on.
        let mut connections: BTreeMap<String, Vec<Connection>> = BTreeMap::new();
        let mut dependencies: Vec<ReadyGate> = Vec::new();
        for (idx, demand) in module.needs().iter().enumerate() {
            let resolved = &chosen[&(module.id().clone(), idx)];
            connections
                .entry(demand.resource.clone())
                .or_default()
                .push(resolved.connection.clone());
            if let Some(gate) = provider_ready_gate(catalog, &resolved.provider) {
                dependencies.push(gate);
            }
        }
        // A consumer that trusts the gateway's CA must not start before it is minted (a JVM
        // reads its truststore once, at startup).
        if let Some(tls) = &gateway.tls {
            let trusts = connections.values().flatten().any(|c| {
                matches!(
                    c,
                    Connection::ObjectStore {
                        tls_trust: Some(_),
                        ..
                    }
                )
            });
            if trusts {
                dependencies.push(ReadyGate {
                    service: tls.mint_service.clone(),
                    readiness: Readiness::CompletedSuccessfully,
                });
            }
        }
        // A *provider* renders against its own role too: the names it provisions (`objects`,
        // for an init block to iterate) plus its own connection resolved for each — so its
        // template reads e.g. `connections.object_store.0.credential` rather than a `${VAR}`.
        let objects = provisioned.get(module.id()).cloned().unwrap_or_default();
        for (role, template) in module.provides().resource_kinds.iter() {
            let role_conns = connections.entry(role.clone()).or_default();
            for name in &objects {
                role_conns.push(template.resolve(name));
            }
        }
        // The gateway module publishes every listener's host port, so dedicated listeners
        // (object stores) are reachable from the host without its template hard-coding a port
        // list.
        let is_gateway = services_of[module.id()]
            .iter()
            .any(|s| s.role == Role::gateway());
        let (published_ports, tls) = if is_gateway {
            (published_ports(gateway, ctx), gateway_render_tls(gateway))
        } else {
            (Vec::new(), None)
        };
        bindings.insert(
            module.id().clone(),
            ModuleBinding {
                knobs: knobs[module.id()].clone(),
                connections,
                dependencies,
                objects,
                published_ports,
                tls,
            },
        );
    }

    Ok(Bound {
        injected,
        bindings,
        env,
    })
}

/// The gateway's published `host:container` mappings: every routing listener, then the Envoy
/// admin endpoint (not a routing listener, so not in `gateway.listeners`), published 1:1.
fn published_ports(gateway: &GatewayConfig, ctx: &PlanCtx) -> Vec<PortMapping> {
    gateway
        .listeners
        .iter()
        .map(|l| PortMapping {
            host: l.host_port,
            container: l.internal_port,
        })
        .chain(std::iter::once(PortMapping {
            host: ctx.gateway_admin_port,
            container: ctx.gateway_admin_port,
        }))
        .collect()
}

/// The gateway module's [`GatewayTls`] render input: the emulated public hostnames (as aliases
/// and certificate names) and, under forward-auth, the secure origin's in-network alias and
/// host-side name. Both draw on the same minted CA, so they share one mint service and volume.
/// `None` when the gateway serves no TLS at all.
fn gateway_render_tls(gateway: &GatewayConfig) -> Option<GatewayTls> {
    let (trust, mint_service) = match (&gateway.tls, &gateway.secure) {
        (Some(t), _) => (&t.trust, &t.mint_service),
        (None, Some(s)) => (&s.trust, &s.mint_service),
        (None, None) => return None,
    };
    let emulated = gateway.tls.iter().flat_map(|t| t.hosts.iter().cloned());
    let hosts: BTreeSet<String> = emulated
        .clone()
        .chain(gateway.secure.iter().map(|s| s.internal_host.clone()))
        .collect();
    let cert_hosts: BTreeSet<String> = emulated
        .chain(gateway.secure.iter().map(|s| s.public_host.clone()))
        .collect();
    Some(GatewayTls {
        hosts: hosts.into_iter().collect(),
        cert_hosts: cert_hosts.into_iter().collect(),
        trust: trust.clone(),
        mint_service: mint_service.clone(),
    })
}
