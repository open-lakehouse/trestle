# Environment `minimal` — gateway http://localhost:9080

## Gateway routes

| Listener | Prefix | Service (cluster) | Upstream | Rewrite |
| --- | --- | --- | --- | --- |
| dedicated :9100 | `/` | `seaweedfs` | seaweedfs:8333 | — |

## Services

| Module | Service | Role | Placement | Endpoints |
| --- | --- | --- | --- | --- |
| `envoy` | `envoy` | `gateway` | container `envoy` | http:10000 |
| `postgres` | `db` | `relational_db` | container `db` | sql:5432 |
| `seaweedfs` | `seaweedfs` | `object_store` | container `seaweedfs` | s3:8333 |
