# Environment `agents-rs` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| shared :9080 | `/api/2.1/unity-catalog` | `unitycatalog` | unitycatalog:8080 | — |
| shared :9080 | `/api/2.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/2.0/mlflow |
| shared :9080 | `/api/3.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/3.0/mlflow |
| shared :9080 | `/api/4.0/mlflow` | `mlflow` | mlflow:5000 | /mlflow/api/4.0/mlflow |
| shared :9080 | `/api/2.0/otel` | `mlflow` | mlflow:5000 | /mlflow |
| shared :9080 | `/ai-gateway` | `mlflow` | mlflow:5000 | /mlflow/gateway |
| shared :9080 | `/uc-mcp/mcp` | `uc-mcp` | uc-mcp:8090 | — |
| shared :9080 | `/omnigent` | `omnigent` | omnigent:8000 | — |
| shared :9080 | `/mlflow` | `mlflow` | mlflow:5000 | — |
| dedicated :9100 | `/` | `rustfs` | rustfs:9000 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `mlflow` | `mlflow` | `experiment_tracking` | container `mlflow` | tracking:5000, tracking_v3:5000, tracking_v4:5000, otel:5000, ai_gateway:5000, ui:5000 |
| `omnigent` | `omnigent` | `agent_platform` | container `omnigent` | ui:8000 |
| `omnigent` | `omnigent-host` | `agent_platform` | container `omnigent-host` | — |
| `postgres` | `db` | `relational_db` | container `db` | sql:5432 |
| `rustfs` | `rustfs` | `object_store` | container `rustfs` | s3:9000 |
| `uc-mcp` | `uc-mcp` | `mcp_server` | container `uc-mcp` | mcp:8090 |
| `unity-catalog-rs` | `unitycatalog` | `data_catalog` | container `unitycatalog` | rest:8080 |
