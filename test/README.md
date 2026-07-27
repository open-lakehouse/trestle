# `trestle env` scenario fixtures

Curated environment manifests for manual smoke-testing and golden render tests.
Each scenario is a source `env.toml` under `scenarios/`; rendered compose stacks
land in `scratch/env/<scenario>/` (gitignored).

## Scenarios (simple → complex)

| Scenario | Modules | What it exercises |
| --- | --- | --- |
| **minimal** | envoy, postgres, seaweedfs | Baseline gateway + relational DB + S3 object store |
| **lakehouse** | minimal + unity-catalog, mlflow | Resource demands (auto-created DBs/buckets) and multi-route gateway |
| **authenticated** | lakehouse + `envoy.auth=true` | Authelia pulled in automatically; forward-auth on API/UI routes |
| **full-azure** | lakehouse modules on Azurite + jaeger, headwaters | Provider substitution (`object_store=azurite`), tracing, lineage UI |

Gateway URL for every scenario: `http://localhost:9080` (dedicated object-store listener: `:9100`).

## Manual workflow

```bash
# List scenarios
just env-scenarios

# Render one into scratch/env/<name>/
just env-render minimal

# Validate compose syntax
just env-validate minimal

# Start (waits for healthchecks) and tear down
just env-up lakehouse
just env-down lakehouse
```

Re-render after changing a scenario manifest or bumping catalog image defaults:

```bash
just env-render lakehouse
```

## Image overrides

Every service image is a module knob (`image`, plus `init_image` / `pgweb_image` where
needed). Bump defaults centrally in
`crates/stack-topology/src/catalog/images.rs`, or override per environment:

```bash
trestle env new myenv --select envoy,postgres,seaweedfs \
  --set postgres.image=postgres:17 \
  --out-dir scratch/env/custom --force
```

Knobs persist in `env.toml` under `[selection.knob_overrides.<module>]`.
Legacy SCREAMING_SNAKE names (`ENVOY_AUTH`, `POSTGRES_IMAGE`, …) are still accepted as
aliases when resolving overrides.

## Configurable today vs structural

**Configurable** (CLI / `env.toml` / knobs):

- Module selection and `object_store` provider preference
- Gateway host/admin ports, dedicated listener base, `data_root`
- `auth`, `serve_ui`, and image knobs (`image`, `init_image`, `pgweb_image`)

**Structural** (not exposed as knobs yet):

- Internal service ports and gateway route prefixes (planner-derived)
- Postgres/S3/Azure/Authelia credentials (typed connection data; changing them
  requires coordinated catalog updates, not a single env var)

## Golden tests

`cargo test -p olai-trestle --test env_render` renders every scenario manifest
and compares deterministic artifacts against `test/expected/<scenario>/`.
Refresh goldens after intentional catalog changes:

```bash
just env-golden-refresh
```
