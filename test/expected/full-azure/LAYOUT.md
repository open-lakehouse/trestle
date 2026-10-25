# Environment `full-azure` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.1/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/api/3.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/3.0/mlflow |
| shared :9080 | `/api/4.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/4.0/mlflow |
| shared :9080 | `/api/v1/lineage` | `headwaters` | headwaters:8091 | /lineage/api/v1/lineage |
| shared :9080 | `/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | /mlflow |
| shared :9080 | `/ai-gateway` | `mlflow` | mlflow:5000 | /mlflow/gateway |
| shared :9080 | `/v1/traces` | `jaeger_otlp_http` | jaeger:4318 | — |
| shared :9080 | `/lineage` | `headwaters` | headwaters:8091 | — |
| shared :9080 | `/jaeger` | `jaeger` | jaeger:16686 | — |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |
| dedicated :9100 | `/` | `azurite` | azurite:10000 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `azurite` | `azurite` | `object_store` | container `azurite` | blob:10000 |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `headwaters` | `headwaters` | `lineage` | container `headwaters` | ui:8091, api:8091 |
| `jaeger` | `jaeger` | `tracing` | container `jaeger` | ui:16686, otlp_grpc:4317, otlp_http:4318 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, tracking_v3:5000, tracking_v4:5000, otel:5000, ai_gateway:5000, ui:5000 |
| `postgres` | `db` | `relational_db` | container `db` | sql:5432 |
| `unity-catalog` | `unitycatalog` | `data_catalog` | container `unitycatalog` | rest:8080, rest_alias:8080 |
