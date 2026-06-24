---
description: What `just up` brings up and how Envoy emulates Databricks URLs
---

# Running locally with compose

`just up` brings up the local platform stack. The active stack depends on the
`local_stack` value chosen at scaffold time; this project was scaffolded with
`local_stack=custom`.

## Active components

- `local-stack-envoy`
- `local-stack-seaweedfs`
- `local-stack-postgres`
- `databricks-emulator-env`
- `frontend-react`
- `ci-github`


## Envoy URL rewrites

| Local URL | Local backend | Databricks equivalent |
|-----------|---------------|----------------------|
| `http://localhost:${ENVOY_PORT}/*` (catch-all) | `golden-path-app-server:8080` | the deployed app |

## Common operations

```bash
just up                # bring up the stack
just up dev            # bring up the dev profile (excludes the app container)
just logs mlflow       # tail logs for one service
docker compose exec db psql -U $POSTGRES_USER

just down              # tear down + wipe volumes
```

## Talking to services

From inside the Rust server (or any other container), services are reachable by
DNS name:

```rust
let mlflow_url = "http://mlflow:5000/mlflow";   // direct
let mlflow_url = std::env::var("MLFLOW_TRACKING_URI")?;  // via Envoy (recommended)
```

The `MLFLOW_TRACKING_URI` form is **the one to use** — it points at Envoy, and
Envoy rewrites the Databricks-shaped `/api/2.0/mlflow/*` path to MLflow's
`/mlflow/api/2.0/mlflow/*`. The same env var on Databricks points at the
workspace.

## Resetting

```bash
just down       # removes all volumes (Postgres data, MLflow runs, S3 buckets)
just up         # fresh start
```