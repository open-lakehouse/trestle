# Running environments: `olai-stack-runtime`

Status: the engine (this page's first half) has landed and `trestle workspace
up/down/list` runs on it. Instances, the layered environment and secret providers
(open-lakehouse/trestle#271) build on the secret-handling decision below.

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

Planning with `PlanCtx::secrets = None`, which keeps the hosted `${VAR}`
references verbatim, works for kinds 1 and 3. Tested on Colima, Compose
interpolates `${VAR}` inside `env_file` contents from its process environment.
It fails for kind 2: the references land literally in `server.properties`,
which Unity Catalog reads as is.

## Decision: credentials reach Compose as process environment only

A runtime instance plans with its secrets **resolved** (`PlanCtx::secrets =
Some`), as today. Materialization then has a mode in which no sensitive value is
written:

- **`env_file`s**: each `KEY=value` line of a sensitive env file is written as
  `KEY=${VAR}`, and `VAR=value` goes into the launch environment. Verified:
  Compose interpolates env-file contents.
- **Compose secrets**: `file: ./modules/<m>/secrets/<name>` becomes
  `environment: VAR`, the file isn't written, and its whole contents go into the
  launch environment. Verified: Compose's `secrets: <name>: environment: VAR`
  delivers the value at `/run/secrets/<name>`.
- **The root `.env`** is not written. Its values go into the launch environment
  directly, where Compose's interpolation finds them first anyway.

The rule is generic over the three kinds above and needs no change per module.
The rendered tree (a cache under `<state>/instances/<id>/`) holds only `${VAR}`
references. The runtime passes the values in `UpOptions::env`, and a sentinel test
in #271 checks that no value appears anywhere in the tree. `trestle env render`
keeps writing ordinary env directories (sensitive files git-ignored) for users who
run `docker compose up` themselves.
