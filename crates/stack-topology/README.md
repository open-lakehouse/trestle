# olai-stack-topology

The environment **topology & addressing** framework for Lakehouse
reference-architecture environments.

Two sibling tools stand up the same kind of multi-service Lakehouse dev
environment — [`trestle`](https://github.com/open-lakehouse/trestle) renders a
standalone project at scaffold time; the
[`hydrofoil`](https://github.com/open-lakehouse/hydrofoil) desktop backend
resolves and regenerates one at runtime. Both must answer the same hard question:

> *What URL does service X use to reach service Y?*

The answer depends on where the caller sits relative to the callee — in the same
process, on the host, or in another container — and on whether traffic goes
through the gateway or direct. Computed ad hoc at each call site, those rules
duplicate and drift (and silently produce wrong addresses for in-container
callers). This crate makes the topology a single, tested model:

- **Role / implementation** — a service declares the *role* it fills
  (`data_catalog`, `object_store`, `gateway`, …) independent of *which*
  implementation fills it, so a catalog can be Unity Catalog today and an Iceberg
  REST Catalog tomorrow without a framework change. **No implementation names
  appear in this crate's types** — they live only in catalog data.
- **Placement / vantage** — where a service runs and where a caller sits.
- **Endpoint / route intent** — what a service offers (port, scheme) and its
  *intent*, declared once and vantage-free. A module declares only intent, never
  its own prefix and never *how* a base path is applied: `Api` (path-agnostic,
  freely rewritable), `UiPrefixable` (a UI that can serve under a base path the
  planner chooses, so self-referential links resolve), or `UiFixed` (can't take a
  base path → needs its own listener). The link-breaking case is unrepresentable.
- **Route plan** — the **coordinator** assigns the actual prefix, rewrite,
  listener, and chosen base path across *all* modules at once (so paths don't
  collide) into a `RoutePlan`. Prefixes are a planning decision, not a module
  decision.
- **Render handshake** — a module's render produces a `RenderOutput` (a compose
  fragment plus zero or more mountable `RenderFile`s); planner-decided values (the
  chosen base path, assigned ports, mount roots) reach the service uniformly via
  compose env-var substitution (`InjectedEnv`), which both command flags and
  mounted config-file contents can read. *How* a UI applies its base path —
  `--static-prefix`, an env var, a config file — is the template's business, not
  this model's.
- **`address(from, to, endpoint, plan, ctx)`** — the single pure function that
  turns all of the above into one concrete `Url`, routing **through the gateway**
  whenever the plan assigns the endpoint a route (the platform's "one unified
  surface" posture). `address_direct(…)` is the explicit escape hatch for the rare
  endpoint that must be reached directly.
- **Surface mode** — the "one unified platform surface" Lakehouse invariant, with
  the in-process desktop variant expressed in-model rather than forked.

The core (model + resolver) is pure: `serde`, `url`, `thiserror`, no I/O and no
templating.
