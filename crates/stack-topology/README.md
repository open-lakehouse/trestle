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

The core (model + resolver) is pure: `serde`, `url`, `thiserror`, no I/O. Rendering is
also pure — it produces artifact strings and relative filenames in memory; the consumer
owns disk I/O — so it is always available and compiles to `wasm32`.

## Deploy targets

Planning is target-neutral: a `Plan` holds each module's typed `ModuleBinding` (knobs,
connections, readiness gates, provisioned objects), never rendered text. A `DeployTarget`
turns it into files:

```rust
let plan = baseline_catalog().plan(&selection, &PlanCtx::default())?; // target: compose
let files = plan.materialize()?;             // render on the plan's own target
let files = plan.materialize_with(&MyTarget)?; // or any `impl DeployTarget`
```

The environment picks its target with `PlanCtx::target` (`[context] target` in `env.toml`,
`trestle env new --target`); Docker Compose (`compose`) is the default and the only
built-in today. A module renders per target — `Module::render_for(&TargetId)`, or a
`DataModule`'s `renders: {compose: …}` map — and the planner rejects a module with no
rendering for the environment's target (`PlanError::UnsupportedOnTarget`) instead of
silently dropping it. The Envoy bootstrap (`render::envoy`) is shared by every target: Envoy
is the gateway implementation, not the deploy target.

## Client environment

Besides the stack itself, every render writes what a *client* (an app, notebook, or
engine) loads to reach it: `client/host.env` for callers on the host and
`client/container.env` for callers on the compose network. Each module contributes
entries through `Provides::client_env`, resolved against the settled routes from the
caller's vantage, so ports, gateway prefixes and base-path rewrites are always the ones
the stack serves:

| Module | Variables |
| --- | --- |
| `unity-catalog` | `UNITY_CATALOG_URL` |
| `mlflow` | `MLFLOW_TRACKING_URI`, `MLFLOW_AI_GATEWAY_URL` |
| `headwaters` | `OPENLINEAGE_URL`, `OPENLINEAGE_ENDPOINT` |
| `jaeger` | `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` |
| `rustfs` | `AWS_ENDPOINT_URL_S3`, `AWS_REGION` |
| `databricks-emulator-env` | `DATABRICKS_HOST`, `DATABRICKS_APP_URL`, forwarded-user contract |
| `omnigent` | `OMNIGENT_URL` |
| `puppygraph` | `PUPPYGRAPH_URL`, `PUPPYGRAPH_BOLT_ADDRESS`, `PUPPYGRAPH_GREMLIN_ADDRESS` (`host:port`) |

The files hold **no credentials**. Clients authenticate through the gateway and read or
write object storage with the temporary credentials Unity Catalog vends. The root `.env`
is different: it only carries values the compose files substitute.

### Python client workspace

Set `Selection::client` (`[selection.client]` in `env.toml`, or `trestle env new --client`)
to also write a [uv](https://docs.astral.sh/uv/) project with [marimo](https://marimo.io)
notebooks into `client/`:

```bash
cd client && uv sync && uv run marimo edit notebooks
```

Notebooks read `host.env` at runtime through marimo's `dotenv` setting, so they are plain
Python with no templating. Modules contribute through `Provides::python`: PEP 508
dependencies, merged into `pyproject.toml`, and example notebooks, written only when
`examples` is set (`--client-examples`). Every workspace gets `00_connect.py`, which checks
each endpoint answers. `pyproject.toml` is regenerated on every render. Notebooks are
written once, so local edits survive.

| Module | Dependencies | Example notebook |
| --- | --- | --- |
| `mlflow` | `mlflow`, `scikit-learn` | `mlflow_quickstart.py`: train a model, track it in MLflow |
| `puppygraph` | `neo4j`, `gremlinpython` | `puppygraph_quickstart.py`: the same graph query in Cypher and Gremlin |

## AI Gateway (MLflow)

The `mlflow` module also serves MLflow's built-in AI Gateway, fronted at `/ai-gateway`
(`MLFLOW_AI_GATEWAY_URL`). Clients pick the provider surface they speak below it:

| Surface | Path | Speaks |
| --- | --- | --- |
| Anthropic | `/ai-gateway/anthropic/v1/messages` | Anthropic Messages (e.g. Claude Code via `ANTHROPIC_BASE_URL=$MLFLOW_AI_GATEWAY_URL/anthropic`) |
| OpenAI | `/ai-gateway/openai/v1/{responses,chat/completions}` | OpenAI Responses / Chat Completions |
| Unified | `/ai-gateway/mlflow/v1/chat/completions` | OpenAI-compatible, any configured provider |

The `model` a client sends is the name of a gateway endpoint. On every `docker compose up`, a
one-shot `mlflow-gateway-init` creates one endpoint per entry in the `mlflow.ai_gateway_models`
knob (`provider:model` pairs, default the current Claude models), each named after the model id
and with usage tracking on. It stores the provider key from the shell that runs `up`:

```bash
export ANTHROPIC_API_KEY=...   # OPENAI_API_KEY / GEMINI_API_KEY for those providers
docker compose up -d
```

Keys are encrypted in MLflow's database and never written into the rendered stack. Clients send
no key of their own. The gateway route drops caller credentials, so MLflow always uses the key
it stores. Endpoints already present are left alone, so changes made in the MLflow UI
(`/mlflow/#/gateway`) survive re-runs.

Upgrading a stack created with an older MLflow image: MLflow refuses an out-of-date schema. Back up
`.data/`, then run `docker compose run --rm mlflow sh -c 'mlflow db upgrade "$MLFLOW_BACKEND_STORE_URI"'`
once (or start from an empty `.data/`).

## Agents (Omnigent)

The `omnigent` module runs the [Omnigent](https://github.com/omnigent-ai/omnigent) agent
platform as part of the stack. Selecting it pulls in `mlflow` for the AI Gateway, plus the
relational and object stores they need.

- **Server.** The web UI and API are fronted at `/omnigent` (`OMNIGENT_URL`), with no request
  timeout so session streams stay open. Its state lives in the stack's Postgres.
- **Agent host** (knob `omnigent.host`, on by default). A container on the stack's network
  that runs the agents. It loads `client/container.env` and forwards every key to the agents,
  so they see `UNITY_CATALOG_URL`, `MLFLOW_TRACKING_URI`, `AWS_ENDPOINT_URL_S3`, the OTLP
  endpoint, and the rest of the selected modules' addresses. Its harnesses (Claude Code, Codex,
  …) call MLflow's AI Gateway (`ANTHROPIC_BASE_URL=$MLFLOW_AI_GATEWAY_URL/anthropic`), so
  the host holds no provider key. `omnigent.model` / `omnigent.fast_model` pick the gateway
  endpoints the harness uses. Export the provider key before `up` so the gateway is seeded:

  ```bash
  export ANTHROPIC_API_KEY=...
  docker compose up -d
  open http://localhost:9080/omnigent/
  ```

- **Your own host.** Agents can also run on your machine against the stack's server:
  `omnigent host "$OMNIGENT_URL"` or `omnigent run --server "$OMNIGENT_URL"`, with
  `client/host.env` loaded.

**Auth.** With the gateway open (the default), Omnigent runs single-user, like the rest of the
stack. With gateway sign-in on, Omnigent trusts the identity the gateway asserts (`Remote-Email`):
its users are the Authelia users, and with policy on the policy applies (chatting needs `writers` or
`admins`). An agent host can't present that identity, so the in-stack host is not rendered
while the gateway authenticates.

## Graph queries (PuppyGraph)

The `puppygraph` module runs [PuppyGraph](https://docs.puppygraph.com), which answers
openCypher and Gremlin queries over Unity Catalog's Delta tables in place, with no copy into a
graph store. Selecting it pulls in `unity-catalog` and the stores it needs. PuppyGraph needs
a lot of memory: give Docker about 8 GB.

- **Schema.** Which tables are nodes and edges lives in `graphs/puppygraph/schema.json`, next
  to `compose.yaml`. It's yours to edit and is written only once. Point each mapping at catalog
  `uc` (the Unity Catalog catalog named by the `puppygraph.catalog` knob, default `unity`):

  ```json
  {
    "node": [{
      "label": "person",
      "dataSourceGroup": {"externalDataSource": {
        "enabled": true, "catalog": "uc", "schema": "default", "table": "person",
        "mappedField": [{"sourceFieldName": "id", "targetFieldName": "id"},
                        {"sourceFieldName": "name", "targetFieldName": "name"}]
      }},
      "id": [{"name": "id", "type": "STRING"}],
      "attribute": [{"name": "name", "type": "STRING"}]
    }],
    "edge": []
  }
  ```

  On every `up`, the one-shot `puppygraph-schema` merges in the generated connection
  (`modules/puppygraph/catalog.json`, the UC metastore) and uploads the result. PuppyGraph
  checks every mapping against the tables before it accepts the schema. While no nodes are mapped it skips the upload, so a
  schema built in the web UI is kept. After editing, run
  `docker compose up puppygraph-schema`.
- **Connections.** PuppyGraph calls Unity Catalog directly on the compose network
  (`puppygraph.uc_url`), not through the gateway, so gateway sign-in doesn't block it. It reads
  the Delta files with temporary credentials UC vends, so it holds no storage keys. Those reads
  go to the AWS S3 hostnames the gateway answers for RustFS, so the container trusts the
  gateway's local CA, in its JVM and in the OS store its native engine uses. That's why it
  starts as root (as all of PuppyGraph's processes do). UC vends nothing for Azurite, so the
  graph can't read table data on an Azure store.
- **Endpoints.** The web UI has its own gateway listener (`PUPPYGRAPH_URL`), because it only
  serves at `/`. Bolt (7687) and Gremlin (8182) are published directly. All three use
  PuppyGraph's login, `puppygraph` / `puppygraph.password` (default `puppygraph123`). Gateway
  sign-in does not cover them, so set your own password if others can reach those ports.
  Omnigent's agents get the addresses through `client/container.env`, like every other
  module's.

## Gateway authentication and authorization

Every environment has the gateway: the planner adds the `gateway`-role provider (Envoy)
whether or not it was selected. How it guards access is a setting of the environment, not
a module to pick:

```toml
[selection.gateway]
auth = true     # sign-in (the `auth`-role provider, Authelia)
policy = true   # authorization (the `policy`-role provider, OPA); implies auth
```

`trestle env new --auth` / `--policy` set these, and the wizard asks for them. Manifests
from before this setting (`[selection.knob_overrides.envoy] auth = "true"`, or `authelia` /
`opa` in `modules`) still plan the same; `Selection::normalize_gateway` rewrites them.

- **Authentication**: `auth = true` fronts every
  API and UI route on the shared listener with Authelia forward-auth. Authelia's
  asserted identity reaches upstreams as `Remote-User` / `Remote-Email` / `Remote-Name` /
  `Remote-Groups`. Client-supplied copies are stripped before any check runs. Accounts
  live in `users.yml` beside `compose.yaml`.
- **HTTPS under auth**: Authelia only issues sessions for a dotted cookie domain on a secure
  origin, so with auth on the shared listener serves `https://<env>.localhost:<port>` from
  the host (browsers resolve `*.localhost` to loopback; no `/etc/hosts` entry needed). The
  certificate comes from the gateway's local CA. Fetch it with
  `docker compose cp gateway-certs:/certs/ca.pem .` and trust it in your browser, or pass
  `curl --cacert ca.pem`. In-network callers keep plain HTTP at the gateway's
  `<env>.internal` network alias. `client/*.env` already points at both. Without auth, the
  gateway stays on `http://localhost:<port>`.
- **Authorization**: `policy = true` adds an Open Policy Agent check after
  authentication (Envoy gRPC `ext_authz`, failing closed), and also turns auth on. The
  policy is `policies/authz.rego`, written once and then yours to edit. By default any
  signed-in user may read and the `writers` or `admins` group may write. Check changes with
  `opa test policies/`, and roll them out with `--set opa.dry_run=true` first: every
  request is allowed and each decision is logged (`docker compose logs opa`).

Resource backends on dedicated listeners (object stores) are never gated.

## Seeing the rendered output

To materialize and print every artifact a selection produces (the Envoy gateway config,
`compose.yaml`, `.env`, the Postgres init script, and each module's compose fragment):

```bash
cargo run -p olai-stack-topology --features std-io --example render_stack
# choose modules (by their short ids):
cargo run -p olai-stack-topology --features std-io --example render_stack -- envoy postgres rustfs jaeger
# prefer Azurite over RustFS for the object_store role:
cargo run -p olai-stack-topology --features std-io --example render_stack -- --azurite
```
