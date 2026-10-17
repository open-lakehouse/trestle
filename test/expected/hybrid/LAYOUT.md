# Environment `hybrid` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.1/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | /mlflow |
| shared :9080 | `/ai-gateway` | `mlflow` | mlflow:5000 | /mlflow/gateway |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `aws-s3` | `s3` | `object_store` | external `https://s3.us-east-1.amazonaws.com` | s3:443 |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `hosted-postgres` | `db` | `relational_db` | external `tcp://lakehouse.abc123example.us-east-1.rds.amazonaws.com:5432` | sql:5432 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, otel:5000, ai_gateway:5000, ui:5000 |
| `unity-catalog` | `unitycatalog` | `data_catalog` | container `unitycatalog` | rest:8080, rest_alias:8080 |

## Hosted resources

Provided outside the stack: README.md walks through creating the listed resources
and setting the secrets before the stack starts.

| Module | Vendor | Cloud | Reached at | Must exist | Secrets |
| --- | --- | --- | --- | --- | --- |
| `aws-s3` | Amazon S3 | aws | `https://s3.us-east-1.amazonaws.com` | `unity`, `mlflow` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` |
| `hosted-postgres` | Amazon RDS | aws | `tcp://lakehouse.abc123example.us-east-1.rds.amazonaws.com:5432` | `unitycatalog`, `mlflow` | `PG_PASSWORD` |
