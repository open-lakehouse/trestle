---
description: Turn the lab into an app harness by dropping in a service alongside the stack
---

# Adding your own service

The lab is designed to host the platform-stack only. To experiment with an
*application* that talks to MLflow / Unity Catalog / Delta from inside the same
compose network:

## 1. Create your service image

The fastest path is a small Python or Rust container that mounts a working
directory:

```yaml
# docker/compose/myapp.yaml
services:
  myapp:
    image: python:3.12-slim
    profiles: [app, svc, full]
    working_dir: /workspace
    volumes:
      - ./myapp:/workspace
    env_file: [.env.local]
    command: ["python", "main.py"]
    depends_on:
      mlflow:
        condition: service_healthy
```

## 2. Wire it to Envoy (optional)

If your service has an HTTP API and you want it on the unified port, add a
route in `docker/envoy/envoy.yaml`:

```yaml
- match: { prefix: "/myapp" }
  route: { cluster: myapp }
```

…and add the `myapp` cluster with `host: myapp, port: 8000`.

## 3. Use the lab's env vars

Your service inherits `.env.local`, so the standard variables work out of the
box:

```python
import os, mlflow
mlflow.set_tracking_uri(os.environ["MLFLOW_TRACKING_URI"])
```

## When to graduate to `databricks-app-rust`

Once your service has more than a handful of endpoints and you want a typed
proto-driven API, scaffold a new project with:

```bash
trestle new my-app --template databricks-app-rust --profile dbx-emulator
```

…which gives you the same compose stack plus a real Axum server, a TypeScript
client, and Databricks Apps deployment.