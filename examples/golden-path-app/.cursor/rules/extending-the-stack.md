---
description: How to add a new service to the lab compose stack
---

# Extending the lab stack

When the user asks to add a new backing service (Redis, OpenSearch, Trino,
Kafka, your own Python or Rust app, ...), follow this checklist.

## Step 1: write the compose fragment

Drop a self-contained file under `docker/compose/<service>.yaml`. The file
must include the `services:` top-level key — Docker Compose `include:` merges
the keys into the parent file.

```yaml
services:
  myservice:
    image: example/myservice:latest
    restart: unless-stopped
    profiles: [svc, full]    # always include `svc` so `just up` brings it up
    environment:
      ...
    healthcheck:
      ...
```

## Step 2: include the fragment in `compose.yaml`

```yaml
include:
  ...existing entries...
  - path: ./docker/compose/myservice.yaml
```

## Step 3: wire it into Envoy (optional)

If users should reach the service via `localhost:$ENVOY_PORT/myservice`, add a
route + cluster to `docker/envoy/envoy.yaml`:

```yaml
# In virtual_hosts[0].routes (above the catch-all):
- match: { prefix: "/myservice" }
  route: { cluster: myservice }

# In clusters[]:
- name: myservice
  type: STRICT_DNS
  load_assignment:
    cluster_name: myservice
    endpoints:
      - lb_endpoints:
          - endpoint:
              address:
                socket_address:
                  address: myservice
                  port_value: 8080
```

## Step 4: add env vars (optional)

If your service introduces new env vars users should be able to customise, add
them to `.env.example` with sensible defaults.

## Step 5: document it

Append a row to the URLs table in `README.md`.

## When to factor it into a trestle component

If you find yourself dropping the same fragment into multiple labs, promote it
to a trestle shared component (in the trestle repo at
`crates/trestle/templates/_components/<name>/`). The manifest's `provides:`
block does all of the above declaratively, so the next lab opts in with one
line.