# Environment `neon-hybrid` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/api/3.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/3.0/mlflow |
| shared :9080 | `/api/4.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/4.0/mlflow |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | /mlflow |
| shared :9080 | `/ai-gateway` | `mlflow` | mlflow:5000 | /mlflow/gateway |
| shared :9080 | `/omnigent` | `omnigent` | omnigent:8000 | — |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `headwaters` | `headwaters` | `lineage` | external `https://br-example-123456-headwaters.compute.eu-central-1.aws.neon.tech` | ui:8091, api:8091 |
| `hosted-postgres` | `db` | `relational_db` | external `tcp://ep-example-123456-pooler.eu-central-1.aws.neon.tech:5432` | sql:5432 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, tracking_v3:5000, tracking_v4:5000, otel:5000, ai_gateway:5000, ui:5000 |
| `neon-storage` | `s3` | `object_store` | external `https://br-example-123456.storage.eu-central-1.aws.neon.tech` | s3:443 |
| `omnigent` | `omnigent` | `agent_platform` | container `omnigent` | ui:8000 |
| `omnigent` | `omnigent-host` | `agent_platform` | container `omnigent-host` | — |
| `unity-catalog-rs` | `unitycatalog` | `data_catalog` | external `https://br-example-123456-unitycatalog.compute.eu-central-1.aws.neon.tech` | rest:8080 |

## Hosted resources

Provided outside the stack: README.md walks through creating the listed resources
and setting the secrets before the stack starts.

| Module | Vendor | Cloud | Reached at | Must exist | Secrets |
| --- | --- | --- | --- | --- | --- |
| `headwaters` | Neon Function | neon | `https://br-example-123456-headwaters.compute.eu-central-1.aws.neon.tech` | — | `HEADWATERS_FUNCTION_URL` |
| `hosted-postgres` | Neon | neon | `tcp://ep-example-123456-pooler.eu-central-1.aws.neon.tech:5432` | `mlflow`, `omnigent` | `PGHOST`, `PGPASSWORD`, `PGUSER` |
| `neon-storage` | Neon | neon | `https://br-example-123456.storage.eu-central-1.aws.neon.tech` | `mlflow` | `NEON_S3_ACCESS_KEY_ID`, `NEON_S3_ENDPOINT`, `NEON_S3_REGION`, `NEON_S3_SECRET_ACCESS_KEY` |
| `unity-catalog-rs` | Neon Function | neon | `https://br-example-123456-unitycatalog.compute.eu-central-1.aws.neon.tech` | — | `UNITYCATALOG_FUNCTION_URL` |
