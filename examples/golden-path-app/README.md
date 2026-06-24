# golden-path-app

A proto-driven Rust + Axum service deployable to **Databricks Apps**, scaffolded
with [trestle](https://github.com/open-lakehouse/trestle).

- Server: `crates/server` (Axum, generated routes)
- Shared models: `crates/common`
- HTTP client: `crates/client`
- Frontend: `frontend/` (React + Vite + TypeScript, generated TS client at `frontend/src/api`)

## Quickstart

```bash
# 1. Set up the env file
cp .env.example .env.local

# 2. Bring up the local platform stack (Postgres, MLflow, Envoy, ...)
just up

# 3. Generate code from .proto and run the app + frontend
just regen
just dev
```

## How the layout maps to Databricks

| Local | Databricks |
|-------|-----------|
| `cargo run -p golden-path-app-server` | `databricks bundle run golden_path_app` || `X-Forwarded-Access-Token` (synthesized by Envoy) | `X-Forwarded-Access-Token` (OBO from Databricks Apps) |
| `DATABRICKS_HOST=http://localhost:${ENVOY_PORT}` | `DATABRICKS_HOST=https://<workspace>` |

The exact same code reads `DATABRICKS_HOST`, `DATABRICKS_TOKEN`, and the OBO
header. No env-conditional branches.

## Layout

```
golden-path-app/
├── Cargo.toml                  # workspace
├── trestle.yaml                # codegen config
├── buf.yaml + buf.gen.yaml     # proto build
├── proto/golden_path_app/v1/    # source of truth
├── crates/
│   ├── common/                 # generated model types
│   ├── server/                 # Axum service (this is what Databricks runs)
│   └── client/                 # HTTP client + shared types
├── frontend/                   # React + Vite
├── app.yaml                    # Databricks Apps manifest
├── databricks.yml              # Databricks Asset Bundle
├── compose.yaml                # local-stack composition
└── docker/                     # compose fragments, envoy config, ...
```

## Commands

```bash
just regen          # rebuild proto descriptor + regenerate Rust + TS
just dev            # cargo run + vite dev (and the local stack)
just up [profile]   # local stack only
just down           # stop local stack
just bundle-validate
just deploy         # databricks bundle deploy + run
just lint           # cargo clippy + buf lint + frontend lint
just test           # cargo test
```

## License

Apache-2.0 — see [LICENSE](LICENSE).