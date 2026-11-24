# Running environments: `olai-stack-runtime`

Status: the engine, the state store, and instances with the layered environment
and secret providers have landed. The CLI runs on them: `trestle ws`, `env
create`/`commit`, `var`, and `up`/`down`/`ps`/`logs`/`health` (see
[`multi-env.md`](multi-env.md)).

`olai-stack-topology` plans and renders an environment and never touches Docker.
`olai-stack-runtime` is the part that runs it, shared by the `trestle` CLI and
the desktop app. The CLI used to shell out to `docker` from
`cli/workspace.rs`; that code now lives here, behind a trait.

## The state store

`olai-stack-state` is the state the CLI and the desktop app share: one SQLite
database, `state.db` under `$TRESTLE_HOME` (default: `trestle` in the platform
data directory, such as `~/Library/Application Support/trestle`). It uses WAL,
and writers take the lock up front.

```text
Workspace ─┬─ vars, mesh
           └─ Environment ─┬─ draft (placement-free env.toml + module.yaml texts + vars), vars, head
                           └─ Revision n (immutable: manifest, modules, vars, digest)
Instance  = a revision to run: desired state, overrides, sources ─── SlotLease (host-wide, 0..=54)
```

- **Revisions are immutable.** `commit_draft` numbers the next one and moves the
  head. The `Revision` type has no setters, and a trigger refuses an `UPDATE`
  even from outside the API.
- **Slots are host-wide.** A `WorkspaceManifest` hands out slots starting at 0 in
  each workspace, so two workspaces clash. The store leases each instance the
  lowest free slot on the host, inside a write transaction, so concurrent
  processes never share one.
- **Secrets are references** (`SecretRef { provider, key }`). Values stay with
  their provider.
- **Schema**: `PRAGMA user_version` with embedded migrations. A newer schema is
  refused (`UnsupportedVersion`), as a newer manifest is.
- **Change feed**: a `Watcher` compares `PRAGMA data_version`, which moves when
  another connection commits, so the app notices a CLI write. It reports that
  something changed and leaves what to the reader.
- **Import/export**: `trestle-workspace.toml` and env directories are the
  exchange format. Import drops slots, because the store leases its own. Export
  writes the chosen instances' slots, so `docker compose up` works in the
  exported directory.

## The engine

`ComposeEngine` is what the runtime asks of Docker Compose:

| Call | Docker CLI |
|---|---|
| `up(dir, opts)` | `docker compose up -d [--wait [--wait-timeout N]]` in `dir` |
| `down(dir, opts)` | `docker compose down [--volumes]` in `dir` |
| `ls()` | `docker compose ls --all --format json` |
| `ps(project)` | `docker compose -p <project> ps --all --format json` |
| `logs(project, opts)` | `docker compose -p <project> logs --timestamps --no-color [--follow] [--tail N]` |
| `events(filter)` | `docker events --format '{{json .}}' --filter type=container --filter label=com.docker.compose.project[=<project>]` |
| `network_rm(name)` | `docker network rm <name>` |

`DockerCli` runs the CLI with `tokio::process`, under the current Docker context
or `DOCKER_HOST` (or a pinned `--context`). It tells "Docker is not available"
(`Error::Unavailable`: no binary, or no daemon) apart from "nothing is running"
(an empty listing). The CLI used to treat both as an empty listing.
`logs` and `events` are streams; dropping one kills its process.

`FakeEngine` (feature `testing`) is an in-memory Docker for tests of code that
drives an engine. Projects are named by the `name:` of their head
`compose.yaml`, the services are scripted, the fake raises the events Docker
would, and every call is recorded with the environment it was given.

Lifecycle calls take the variables to add to Compose's process environment
(`UpOptions::env`). That is how credentials reach Compose; see below.

Helpers:

- `probe_host_ports(&Plan)` binds each of `Plan::host_ports` on `0.0.0.0` to find
  the ones something already holds, so a start can be refused before Compose
  fails half-way. This is a check, not a reservation.
- `managed_projects(engine, ProjectMatch)` picks the projects a caller manages,
  by exact names or a prefix. Workspace projects are `<workspace>-<env>` and both
  parts may contain dashes, so a `demo-` prefix also matches a workspace called
  `demo-x`. Match workspace members by their exact names. Store instances
  (#271) use `_` as the separator, which trestle names can't contain.
- `remove_mesh_if_idle` removes a workspace's mesh network once none of its
  members is known to Docker. Compose labels a network with the project that
  created it, so no member's own `down` removes a network that others joined.

## What the engines print

`scripts/runtime-capture/capture.sh <context>` runs a three-service project
(`scripts/runtime-capture/compose.yaml`) through everything the runtime parses:
- a healthy `up --wait`, and one with a service that never turns healthy
- `ls` and `ps` while running, partly stopped and down
- `logs`
- the event stream throughout

The output is committed under `crates/stack-runtime/tests/fixtures/<engine>/`,
and every fixture test runs on each engine directory.

| Engine | Docker / Compose | Captured |
|---|---|---|
| Colima (Ubuntu 24.04 VM) | Engine 29.5.2, Compose 5.6.1 | yes |
| OrbStack | | not yet captured |
| Docker Desktop | | not yet captured |

To add an engine, run the script against its context
(`capture.sh orbstack`) and commit the new directory; the tests pick it up.

What the capture showed:

- `compose ls --format json` prints one JSON array. Its `Status` is a summary
  per state, such as `exited(1), running(2)`, which the runtime parses into
  counts. `--all` lists stopped projects too.
- `compose ps --format json` prints **one JSON object per line**. The parser also
  accepts an array, which some Compose versions print. `Health` is
  `healthy`/`unhealthy`/`starting`, or empty without a healthcheck. `Publishers`
  lists every exposed port, with `PublishedPort: 0` for an unpublished one.
- Events carry the project and service as container labels. Every healthcheck
  probe raises `exec_create`/`exec_start`/`exec_die`, which made up about 90% of
  the stream, and those are dropped. Health changes arrive as
  `health_status: healthy|unhealthy`, and `die` carries `exitCode`.
- `up --wait` treats a service without a healthcheck as ready once it runs.
  With an unhealthy service it exits 1 and prints
  `container <name> is unhealthy`; the containers stay up.
- `logs --timestamps --no-color` lines read `<service>-<n>  | <RFC 3339> <text>`.

## Where credentials land

The spike rendered every baseline module with a sentinel in each `LOCAL_*`
credential and each hosted secret, then looked for the sentinels in the output.
Every credential ends up in a file:

| Credential | Files |
|---|---|
| `LOCAL_POSTGRES_PASSWORD` | `modules/postgres/secrets/postgres_password` (compose secret), `modules/unity-catalog-oss/secrets/hibernate.properties` (compose secret), the `env_file`s of mlflow, headwaters (+ migrate) and omnigent |
| `LOCAL_RUSTFS_ACCESS_KEY`, `LOCAL_RUSTFS_SECRET_KEY` | `modules/rustfs/.env/rustfs.env`, `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog-oss/secrets/server.properties` |
| `LOCAL_AUTHELIA_*` (3) | `modules/authelia/secrets/<name>` (compose secrets) |
| Hosted `${PG_PASSWORD}` | `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog-oss/secrets/hibernate.properties` |
| Hosted `${AWS_ACCESS_KEY_ID}`, `${AWS_SECRET_ACCESS_KEY}` | `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog-oss/secrets/server.properties`, and the root `.env` |

Each of these files is marked `sensitive` and git-ignored, and each is one of
three kinds:

1. a module **`env_file`** (`modules/<m>/.env/<name>.env`);
2. a **compose secret** mounted from a file (`secrets: <name>: file:
   ./modules/<m>/secrets/<name>`), including Unity Catalog's whole
   `server.properties` / `hibernate.properties`;
3. the root **`.env`**, which Compose reads for `${VAR}` interpolation.

Planning with `PlanCtx::secrets = None` keeps the hosted `${VAR}` references
verbatim, and that only goes so far:

- Compose interpolates `${VAR}` inside `env_file` contents from its process
  environment (tested on Colima). trestle's fragments read their env files with
  `format: raw`, though, which turns interpolation off.
- Kind 2 doesn't work at all: the references land literally in
  `server.properties`, which Unity Catalog reads as is.

## Decision: credentials reach Compose as process environment only

A runtime instance plans with its secrets **resolved** (`PlanCtx::secrets =
Some`), as today. Materialization then has a mode in which no sensitive value is
written:

- **`env_file`s**: each `KEY=value` line of a sensitive env file is written as
  `KEY=${VAR}`, and `VAR=value` goes into the launch environment. The fragments
  referencing the file drop its `format: raw`. Every value is now a reference,
  so the raw format has nothing left to protect, and Compose has to interpolate
  the file. Verified: Compose interpolates env-file contents.
- **Compose secrets**: `file: ./modules/<m>/secrets/<name>` becomes
  `environment: VAR`, the file isn't written, and its whole contents go into the
  launch environment. Verified: Compose's `secrets: <name>: environment: VAR`
  delivers the value at `/run/secrets/<name>`.
- **The root `.env`** is not written. Its values go into the launch environment
  directly, where Compose's interpolation finds them first anyway.

This is `MaterializedOutput::into_launch_env` in `olai-stack-topology`. The rule
is generic over the three kinds above and needs no change per module. Any other
sensitive file is an error, not something left behind. The rendered tree (a cache
under `<state>/instances/<id>/`) holds only `${VAR}` references, and the runtime
passes the values in `UpOptions::env`. Two sentinel tests check that no value
appears anywhere in the tree: every baseline module with sentinel credentials,
and a hosted plan. Instances are tested the same way, under `FakeEngine` and
against real Docker. `trestle env render` keeps writing ordinary env directories
(sensitive files git-ignored) for users who run `docker compose up` themselves.

## Decision: egress credentials reach an in-environment secret server

The credentials an egress gateway injects into outbound requests (an agent's
API key for an outside MCP server, say) follow the rule above, with one
narrowing: only the environment's `secret-server` service lists them. It hands
them to the egress Envoy over SDS, through a Unix socket in a volume only that
Envoy mounts, so they reach no other container and still never land in the
rendered tree. Rotating one recreates the secret server alone. A host-backed
source (the keychain, read live) is a later phase. See
[`egress-gateway.md`](egress-gateway.md) for the design and why the server
runs inside the environment.

The runtime needs nothing route-specific for this. The egress module declares
each route's secret as a required secret (`Plan::declared_secrets`), so it
resolves like any other declared secret: from the instance's layers, else from
the `env` provider by name. A route whose secret is set nowhere fails `start`
with `InstanceError::MissingSecrets` before Compose runs. The secret goes into
the launch environment with everything else, and the `secret-server` service
lists it by name only (`environment: VAR:` with no value), so Compose hands it
to that container and no other. Tests check this with sentinel values: under
`FakeEngine`, the value is in no file of the instance's tree and only
`secret-server` names its variable in the rendered compose files. Against real
Docker, a consumer gets a `200` through the egress gateway with the injected
token in place of its own, only the `secret-server` container's environment
holds the value, and after a rotation the next request carries the new value
from the same egress container.

Rotation (`trestle secret rotate`, see the [CLI](../crates/trestle/README.md))
stores the new value where `trestle var set --secret` would, then calls
`Instances::recreate_service(id, "secret-server")` for each running instance.
Envoy's SDS stream drops and reconnects, the new version arrives, and Envoy
swaps it in without a restart. While the server is down, Envoy keeps the
value it had.

### Keyrings

A module that seals data under a key the secret server holds
(`Provides::keys`) gets the same treatment, with a keyring in place of a token.
The planner derives the keyring's variable,
`TRESTLE_KEYRING_<MODULE>__<KEY>` (`olai_stack_topology::plan::keyring_var`),
and declares it as required on the egress module alone. So it resolves, fails
`start` when set nowhere, and reaches only the `secret-server` container like
a route's secret. The module gets its socket (`TRESTLE_SDS_SOCKET`) and the
key's name (`TRESTLE_KEY_<KEY>`), never the keyring.

`olai_stack_runtime::Keyring` is the value: the JSON the secret server reads
(`{versions, active, min_decrypt, aliases}`), with the operations the CLI
offers on it:

- `Keyring::import(base64, alias)` makes version 1 from an existing 32-byte
  key, such as `UC_SERVER_KEK` with the alias `kek-1`.
- `Keyring::rotate` adds a random version (from the OS generator) and makes it
  active.
- `Keyring::retire_below(n)` raises `min_decrypt`. It refuses `n` above the
  active version.
- `Keyring::summary` shows versions, `active`, `min_decrypt` and aliases, never
  material.

**Keyrings are add-only.** No operation removes a version's material. A retired
version stops unwrapping but keeps its key, so lowering `min_decrypt` again
recovers a blob the sweep missed. Before a caller stores a keyring in place of
the stored one, it calls `Keyring::check_extends`, which refuses a write that
would drop or replace a version, or drop or move an alias. `Keyring`'s `Debug`
shows no material, and its JSON form is a `SecretValue`.

Rotating a keyring stores the new value and recreates `secret-server`, as for a
token. The consumer's `KeyServiceProvider` keeps running: its `WatchKey` stream
reconnects and picks up the new active version.

Tests:

- Sentinel keys under `FakeEngine` (`tests/egress_instances.rs`): no file in
  the instance's tree holds a key, its base64 or the keyring, and only
  `secret-server` names the variable.
- `tests/keyring_server.rs` checks that the secret server parses every keyring
  `Keyring` writes and honours its versions.
- The live test (`tests/live_keys.rs`, `--ignored`) runs a module process (the
  `key_probe` example) through an import, a rotation, the `rewrap_all` sweep
  and a retirement on real Docker.

## Instances

`Instances` runs the store's instances on an engine. Each instance runs one
revision, so two revisions of one environment run side by side:

| | Value | Why |
|---|---|---|
| Compose project | `<workspace>_<env>_r<n>` | Trestle names can't contain `_`, so the name parses back to its parts, and reconcile tells instance projects from any other. |
| Host label | `<workspace>-<env>-r<n>` | Its own `*.localhost` sign-in cookie. |
| Slot | leased from the store, host-wide | Its own host ports. |
| Mesh | `trestle-<workspace>-mesh` when the workspace has one | |

`start` does the following:
1. Leases the instance a slot.
2. Refuses a second running instance of the same revision, since the two would
   share a project.
3. Renders the revision afresh into `<state>/instances/<id>/` (mode 0700) and
   checks the plan's host ports.
4. Records the instance as wanted running and runs `up --wait` with the launch
   environment.

`recreate_service(id, service)` recreates one service of a running instance
with the launch environment it resolves to now (`docker compose up -d --no-deps
--force-recreate --wait <service>`); every other container keeps running. It
renders the tree afresh as `start` does, fails with `MissingSecrets` before
Compose when a required secret is set nowhere, and refuses an instance that
isn't running (`NotRunning`) or a service its plan doesn't run
(`UnknownService`).

`stop` runs `down` and keeps the slot. `remove` also drops the tree and
releases the slot. `health` and `events` follow the engine. `reconcile`, run on
startup, adopts running instance projects, marks instances whose project is
gone as stopped, and reports, or takes down, instance-shaped projects that the
store doesn't know.

### Instances of a team's store

An instance names the store that holds its environment: the local one, or a team
store attached on this machine (`olai-stack-state`, feature `postgres`).
`Instances::with_attached` adds attached stores to the one it was made with. An
instance whose store isn't open fails with `InstanceError::StoreNotOpen`. While
any exists, `reconcile` lists them as `unresolved` and doesn't take orphans down,
since one may be theirs.

With feature `neon`, a team's store lives in the `trestle_state` database on its
Neon project. `neon::init_state` creates and migrates it, `neon::attach` records it
on a machine, and `neon::open_attached` opens it with the user's own Neon CLI
login. No credential is stored.

### The layered environment

`EffectiveEnv` resolves an instance's variables from these layers, lowest
precedence first:

1. catalog defaults: the in-stack development credentials (`LOCAL_*`)
2. the revision manifest: the plan's injected variables
3. placement: `COMPOSE_PROJECT_NAME`, `TRESTLE_INSTANCE`, `TRESTLE_SLOT` and
   `TRESTLE_HOST_LABEL`, which no layer can override
4. the plan's client environment
5. workspace vars
6. environment vars
7. revision vars
8. local vars: this machine's own (`VarScope::Local`), never shared with a team,
   such as minted keys and the user's own API keys
9. source overlays, in the order the instance binds them
10. instance overrides

Secret references resolve last, once every layer has had its say. Each variable
keeps its provenance and the layers it overrode, and `masked()` is the view for
a UI. A key that any layer holds as a secret stays masked, even where a higher
layer sets it as a plain literal. The same is true of a plan variable that carries
a resolved credential under a key of its own. The resolution also feeds the
launch: the instance plans with these values as `PlanCtx::secrets` and
`LOCAL_*` credentials, and Compose gets them as process environment.

Planning needs the credentials, and the plan contributes layers 2 and 4. So the
runtime resolves twice: first without the plan's layers, to plan, and then with
them.

### Secret providers and sources

A `SecretRef { provider, key }` resolves through a `SecretProvider`:

- `EnvProvider` (`env`) reads the process environment.
- `KeychainProvider` (`keychain`, feature `keychain`) reads the OS credential
  store: macOS Keychain, Windows Credential Manager, or the Linux kernel keyring.

A reference to a provider that isn't configured is an error. Nothing falls back
to another provider.

Modules declare the secrets their containers read (`Plan::declared_secrets`,
names only). At materialize, one the instance's layers don't set comes from the
`env` provider by name, when it is registered. A required one set nowhere is
`InstanceError::MissingSecrets`: `start` fails before Compose runs, while `stop`
still works (`Materialized::missing_secrets` lists them).

A `Source` overlays an instance with variables, endpoints and credentials from
outside the store, such as a Neon branch (#274). Its credentials may expire
(`Overlay::expires_at`), and an expired overlay is refreshed before use.
`UserVars` (`user`) is built in. An instance binds each source it takes with a
config (`SourceBinding { name, config }`, e.g. which branch), and a workspace
can set a config per source for all its instances (e.g. which project). The
source gets the binding's keys merged over the workspace's
(`SourceCtx::config`). Neither holds secrets. An instance that binds a source
nobody configured fails to start.

### Neon branches

Feature `neon` adds a Neon branch as a source, all through the Neon CLI (see
[`neon.md`](neon.md) for what it prints):

- `NeonControl` is what trestle asks of Neon: branches with their annotations,
  create, delete and reset, connection details, databases, functions, storage,
  buckets, and storage credentials. `NeonCli` runs `neon` (or `neonctl`) with
  `--output json`. It is found at `$NEON_CLI`, on `PATH`, in the Homebrew prefixes
  and `~/.npm-global/bin`, or through a login shell, and it parses the first JSON
  value of the output. `FakeNeon` (with `testing`) is an in-memory project that
  behaves as the spike found the real one does, and can be seeded from the
  fixtures.
- `bind` checks a branch against `Plan::neon_requirements()`. A missing function
  is an error at bind time, not at start. It creates the databases and buckets
  the branch lacks and mints a storage credential on it for the instance; the
  caller keeps the secret. The result, a `NeonBinding`, is the instance's
  `neon` source config. For a plan that calls the branch's AI Gateway,
  `bind_ai_gateway` checks it is enabled and mints an `ai_gateway:invoke`
  credential; the caller keeps its token too. `unbind` revokes the credentials. A
  child branch inherits its parent's credentials, so every binding mints its own
  rather than reuse one.
- `NeonBranchSource` overlays the instance with `PGHOST`, `PGUSER`,
  `PGDATABASE`, `PGPASSWORD`, `DATABASE_URL` (pooled), `DATABASE_URL_UNPOOLED`,
  the `NEON_S3_*` storage coordinates, each function's `<SLUG>_FUNCTION_URL`, and
  `NEON_AI_GATEWAY_BASE_URL` and `NEON_AI_GATEWAY_TOKEN` when bound to the
  gateway. The credentials are masked wherever the environment is shown.
- Branches trestle creates carry the `trestle.owner` annotation and an expiry,
  and only those are reset or deleted.
- `stop` falls back to the tree rendered at the last start when a source fails,
  so a Neon outage never keeps an instance from stopping.
