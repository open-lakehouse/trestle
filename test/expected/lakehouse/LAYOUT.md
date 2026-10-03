# Environment `lakehouse` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.1/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | — |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |
| dedicated :9100 | `/` | `rustfs` | rustfs:9000 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, otel:5000, ui:5000 |
| `postgres` | `db` | `relational_db` | container `db` | sql:5432 |
| `rustfs` | `rustfs` | `object_store` | container `rustfs` | s3:9000 |
| `unity-catalog` | `unitycatalog` | `data_catalog` | container `unitycatalog` | rest:8080, rest_alias:8080 |
