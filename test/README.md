# `trestle env` scenario fixtures

Curated environment manifests for manual smoke-testing and golden render tests.
Each scenario is a source `env.toml` under `scenarios/`; rendered compose stacks
land in `scratch/env/<scenario>/` (gitignored).

## Scenarios (simple → complex)

| Scenario | Modules | What it exercises |
| --- | --- | --- |
| **minimal** | envoy, postgres, rustfs | Baseline gateway + relational DB + S3 object store (with STS) |
| **lakehouse** | minimal + unity-catalog, mlflow | Resource demands (auto-created DBs/buckets), multi-route gateway, UC S3 credential vending |
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

## S3 credential vending (RustFS + emulated AWS)

Unity Catalog OSS has no S3 or STS endpoint override: it always calls
`https://sts.<region>.amazonaws.com` to vend credentials, and its clients then call
`https://<bucket>.s3.<region>.amazonaws.com`. When `rustfs` is the object store, the gateway
answers those names inside the compose network:

```
unitycatalog ──https://sts.us-east-1.amazonaws.com──▶ envoy :443 ──▶ sts-shim ──▶ rustfs
in-network client ──https://unity.s3.us-east-1.amazonaws.com──▶ envoy :443 ──▶ rustfs
host client ──http://localhost:9100 (AWS_ENDPOINT_URL)──▶ envoy :9100 ──▶ rustfs
```

- The gateway holds every emulated hostname as a network alias (one per bucket, since
  Compose DNS has no wildcards) and terminates TLS with a CA minted by the one-shot
  `gateway-certs` service into the `gateway-certs` volume.
- UC gets a real-AWS `server.properties` (bucket, region, role ARN, keys; no endpoint) and
  trusts the CA through `JAVA_TOOL_OPTIONS`. Other in-network clients that should use the
  AWS names must trust it too: `AWS_CA_BUNDLE=/certs/ca.pem` (botocore) or
  `SSL_CERT_FILE=/certs/ca.pem` (object_store, obstore, delta-rs), with the volume mounted.
- `sts-shim` exists because RustFS rejects UC's AssumeRole session policy (mid-word action
  wildcards such as `s3:GetO*`, and `kms:*` condition keys). It expands the wildcards, drops
  the KMS statements, and re-signs. RustFS then enforces the policy, so vended credentials
  stay inside the volume or table prefix.
- Envoy logs one line per request (`<host> <method> <path> -> <cluster> <status>`). A request
  missing from `docker compose logs envoy` went somewhere else, possibly real AWS.

End-to-end check (UC vends write/read credentials; writes outside the prefix are denied):

```bash
just env-render lakehouse && just env-up lakehouse
just env-smoke-s3
```

## Image overrides

Every service image is a module knob (`image`, plus `init_image` / `pgweb_image` where
needed). Bump defaults centrally in
`crates/stack-topology/src/catalog/images.rs`, or override per environment:

```bash
trestle env new myenv --select envoy,postgres,rustfs \
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
- `auth`, `serve_ui`, and image knobs (`image`, `init_image`, `pgweb_image`, `sts_shim_image`, `certs_image`)

**Structural** (not exposed as knobs yet):

- Internal service ports and gateway route prefixes (planner-derived)
- Postgres/S3/Azure/Authelia credentials (typed connection data; changing them
  requires coordinated catalog updates, not a single env var)

## Runtime environment and secrets

Rendered stacks keep container configuration out of Compose YAML:

- Per-service variables are written under `modules/<module>/.env/` and loaded with
  `env_file` using Compose's raw parser, so URLs and connection strings are not
  reinterpreted.
- Postgres and Authelia use top-level Compose `secrets` plus their native `*_FILE`
  variables. Other images without a file-secret contract keep credentials in ignored
  service env files.
- Generated `.env`, module `.env/` and `secrets/` directories, and `.data/` are covered by
  the generated environment-root `.gitignore`. Sensitive files are written with mode `0600`
  on Unix.
- Operator-owned files such as Authelia's `users.yml` are written at the environment root
  (beside `compose.yaml`) and are **not** overwritten on subsequent renders. Delete the
  file and re-render to restore the catalog default.

An env file prevents credentials from landing in committed YAML; it does not hide them from
the container process environment. Commands such as `docker compose config` may display the
resolved environment, so treat that output as sensitive.

## Golden tests

`cargo test -p olai-trestle --test env_render` renders every scenario manifest
and compares deterministic non-sensitive artifacts against `test/expected/<scenario>/`.
Sensitive env and secret files are generated only in temporary directories and checked
structurally (existence, references, ignore rules, and file permissions).
Refresh goldens after intentional catalog changes:

```bash
just env-golden-refresh
```
