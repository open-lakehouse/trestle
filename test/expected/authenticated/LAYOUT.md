# Environment `authenticated` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.1/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | — |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |
| dedicated :9100 | `/` | `seaweedfs` | seaweedfs:8333 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `authelia` | `authelia` | `auth` | container `authelia` | http:9091 |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, otel:5000, ui:5000 |
| `postgres` | `db` | `relational_db` | container `db` | sql:5432 |
| `seaweedfs` | `seaweedfs` | `object_store` | container `seaweedfs` | s3:8333 |
| `unity-catalog` | `unitycatalog` | `data_catalog` | container `unitycatalog` | rest:8080, rest_alias:8080 |
