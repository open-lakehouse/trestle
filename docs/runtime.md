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
| `LOCAL_POSTGRES_PASSWORD` | `modules/postgres/secrets/postgres_password` (compose secret), `modules/unity-catalog/secrets/hibernate.properties` (compose secret), the `env_file`s of mlflow, headwaters (+ migrate) and omnigent |
| `LOCAL_RUSTFS_ACCESS_KEY`, `LOCAL_RUSTFS_SECRET_KEY` | `modules/rustfs/.env/rustfs.env`, `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog/secrets/server.properties` |
| `LOCAL_AUTHELIA_*` (3) | `modules/authelia/secrets/<name>` (compose secrets) |
| Hosted `${PG_PASSWORD}` | `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog/secrets/hibernate.properties` |
| Hosted `${AWS_ACCESS_KEY_ID}`, `${AWS_SECRET_ACCESS_KEY}` | `modules/mlflow/.env/mlflow.env`, `modules/unity-catalog/secrets/server.properties`, and the root `.env` |

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

`stop` runs `down` and keeps the slot. `remove` also drops the tree and
releases the slot. `health` and `events` follow the engine. `reconcile`, run on
startup, adopts running instance projects, marks instances whose project is
gone as stopped, and reports, or takes down, instance-shaped projects that the
store doesn't know.

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
8. source overlays, in the order the instance binds them
9. instance overrides

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

A `Source` overlays an instance with variables, endpoints and credentials from
outside the store, such as a Neon branch (#274). Its credentials may expire
(`Overlay::expires_at`), and an expired overlay is refreshed before use.
`UserVars` (`user`) is built in. An instance binds each source it takes with a
config (`SourceBinding { name, config }`, e.g. which branch), and a workspace
can set a config per source for all its instances (e.g. which project). The
source gets the binding's keys merged over the workspace's
(`SourceCtx::config`). Neither holds secrets. An instance that binds a source
nobody configured fails to start.
