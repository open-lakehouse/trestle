// Auto-generated fixture from the real planner (crates/stack-topology-wasm).
// Regenerate: see node/stack-wasm/README.md. Do not hand-edit.

import { FIXTURE_FILES } from "./fixture-files";
import type { CatalogDto, PlanResult } from "./types";

export const FIXTURE_CATALOG: CatalogDto = {
  modules: [
    {
      id: "envoy",
      display_name: "Envoy gateway",
      summary: "Single-port gateway, Databricks-shaped URL rewrites.",
      category: "gateway",
      provider_of: "gateway",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "auth",
          title: "Require authentication",
          kind: {
            kind: "bool",
          },
          default: "false",
          required: false,
          help: "Front API and UI routes with Authelia single-sign-on (forward-auth). Resource backends (object stores, databases) are never gated.",
          aliases: ["ENVOY_AUTH"],
        },
        {
          key: "image",
          title: "Envoy image",
          kind: {
            kind: "string",
          },
          default: "envoyproxy/envoy:v1.34-latest",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["ENVOY_IMAGE"],
        },
        {
          key: "certs_image",
          title: "Certificate minting image",
          kind: {
            kind: "string",
          },
          default: "eclipse-temurin:17-jdk-noble",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["GATEWAY_CERTS_IMAGE"],
        },
      ],
    },
    {
      id: "authelia",
      display_name: "Authelia",
      summary: "Forward-auth single-sign-on for the gateway (file-based).",
      category: "gateway",
      provider_of: "auth",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "Authelia image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/authelia/authelia:4.39",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["AUTHELIA_IMAGE"],
        },
      ],
    },
    {
      id: "postgres",
      display_name: "Postgres",
      summary: "Postgres 16; auto-creates DBs other modules declare.",
      category: "metadata_db",
      provider_of: "relational_db",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "Postgres image",
          kind: {
            kind: "string",
          },
          default: "postgres:16",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["POSTGRES_IMAGE"],
        },
        {
          key: "pgweb_image",
          title: "pgweb image",
          kind: {
            kind: "string",
          },
          default: "sosedoff/pgweb:latest",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["PGWEB_IMAGE"],
        },
      ],
    },
    {
      id: "rustfs",
      display_name: "RustFS (local S3 + STS)",
      summary:
        "Self-hosted S3-compatible object store with STS; answers the AWS S3/STS hostnames in-network.",
      category: "storage",
      provider_of: "object_store",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "RustFS image",
          kind: {
            kind: "string",
          },
          default: "rustfs/rustfs:1.0.1",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["RUSTFS_IMAGE"],
        },
        {
          key: "init_image",
          title: "RustFS init image",
          kind: {
            kind: "string",
          },
          default: "amazon/aws-cli:latest",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["RUSTFS_INIT_IMAGE"],
        },
        {
          key: "sts_shim_image",
          title: "STS shim image",
          kind: {
            kind: "string",
          },
          default: "python:3.13-alpine",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["STS_SHIM_IMAGE"],
        },
      ],
    },
    {
      id: "azurite",
      display_name: "Azurite (local Azure Blob)",
      summary: "Azure Blob emulator; object store with Azure-shaped wiring.",
      category: "storage",
      provider_of: "object_store",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "Azurite image",
          kind: {
            kind: "string",
          },
          default: "mcr.microsoft.com/azure-storage/azurite:3.35.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["AZURITE_IMAGE"],
        },
        {
          key: "init_image",
          title: "Azurite init image",
          kind: {
            kind: "string",
          },
          default: "mcr.microsoft.com/azure-cli:2.87.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["AZURITE_INIT_IMAGE"],
        },
      ],
    },
    {
      id: "mlflow",
      display_name: "MLflow tracking",
      summary: "Experiment + model tracking; Databricks-shaped URLs.",
      category: "ml",
      provider_of: "experiment_tracking",
      requires: ["envoy"],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "MLflow image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/mlflow/mlflow:v3.10.1-full",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["MLFLOW_IMAGE"],
        },
      ],
    },
    {
      id: "unity-catalog",
      display_name: "Unity Catalog",
      summary: "Databricks UC server; Databricks-shaped REST API.",
      category: "catalog",
      provider_of: "data_catalog",
      requires: ["envoy"],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "Unity Catalog image",
          kind: {
            kind: "string",
          },
          default: "unitycatalog/unitycatalog:v0.6.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["UC_IMAGE"],
        },
      ],
    },
    {
      id: "jaeger",
      display_name: "Jaeger tracing",
      summary: "All-in-one OTLP tracing backend with the Jaeger UI.",
      category: "observability",
      provider_of: "tracing",
      requires: ["envoy"],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "Jaeger image",
          kind: {
            kind: "string",
          },
          default: "cr.jaegertracing.io/jaegertracing/jaeger:2.14.1",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["JAEGER_IMAGE"],
        },
      ],
    },
    {
      id: "headwaters",
      display_name: "Headwaters lineage",
      summary:
        "OpenLineage lineage service; Databricks-style API + optional UI.",
      category: "observability",
      provider_of: "lineage",
      requires: ["envoy"],
      conflicts_with: [],
      knobs: [
        {
          key: "serve_ui",
          title: "Serve the lineage UI",
          kind: {
            kind: "bool",
          },
          default: "true",
          required: false,
          help: "Serve the bundled lineage web UI. Turn off to run the service API-only (e.g. when embedding a custom UI built on the shipped components).",
          aliases: ["HEADWATERS_SERVE_UI"],
        },
        {
          key: "image",
          title: "Headwaters image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/open-lakehouse/headwaters:0.0.7",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["HEADWATERS_IMAGE"],
        },
      ],
    },
    {
      id: "databricks-emulator-env",
      display_name: "Databricks app runtime contract",
      summary: "DATABRICKS_HOST / TOKEN / forwarded-user env vars apps expect.",
      category: "app_runtime",
      provider_of: "databricks_apps_contract",
      requires: [],
      conflicts_with: [],
      knobs: [],
    },
  ],
  default_selection: {
    modules: ["envoy", "postgres", "rustfs"],
    capabilities: [],
    knob_overrides: {},
    extra_resources: [],
  },
};

export const FIXTURE_PLAN: PlanResult = {
  graph: {
    edges: [
      {
        condition: "service_healthy",
        from: "mlflow",
        kind: "startup",
        to: "db",
      },
      {
        condition: "service_healthy",
        from: "mlflow",
        kind: "startup",
        to: "rustfs",
      },
      {
        condition: "service_healthy",
        from: "pgweb",
        kind: "startup",
        to: "db",
      },
      {
        condition: "service_healthy",
        from: "sts-shim",
        kind: "startup",
        to: "rustfs",
      },
      {
        condition: "service_healthy",
        from: "unitycatalog",
        kind: "startup",
        to: "db",
      },
      {
        condition: "service_healthy",
        from: "unitycatalog",
        kind: "startup",
        to: "rustfs",
      },
      {
        from: "envoy",
        kind: "route",
        routes: [
          {
            gated: false,
            host_port: 9080,
            prefix: "/api/2.1/unity-catalog",
            rewrite: null,
          },
          {
            gated: false,
            host_port: 9080,
            prefix: "/unity-catalog",
            rewrite: null,
          },
        ],
        to: "unitycatalog",
      },
      {
        from: "envoy",
        kind: "route",
        routes: [
          {
            gated: false,
            host_port: 9080,
            prefix: "/api/2.0/mlflow",
            rewrite: "/mlflow/api/2.0/mlflow",
          },
          {
            gated: false,
            host_port: 9080,
            prefix: "/api/2.0/otel",
            rewrite: null,
          },
          {
            gated: false,
            host_port: 9080,
            prefix: "/mlflow",
            rewrite: null,
          },
        ],
        to: "mlflow",
      },
      {
        from: "envoy",
        kind: "route",
        routes: [
          {
            gated: false,
            host_port: 9080,
            prefix: "/jaeger",
            rewrite: null,
          },
        ],
        to: "jaeger",
      },
      {
        from: "envoy",
        kind: "route",
        routes: [
          {
            gated: false,
            host_port: 9100,
            prefix: "/",
            rewrite: null,
          },
        ],
        to: "rustfs",
      },
      {
        from: "unitycatalog",
        hosts: [
          "mlflow.s3.amazonaws.com",
          "mlflow.s3.us-east-1.amazonaws.com",
          "s3.amazonaws.com",
          "s3.us-east-1.amazonaws.com",
          "sts.amazonaws.com",
          "sts.us-east-1.amazonaws.com",
          "unity.s3.amazonaws.com",
          "unity.s3.us-east-1.amazonaws.com",
        ],
        kind: "emulated",
        port: 443,
        to: "envoy",
      },
      {
        from: "envoy",
        hosts: ["sts.amazonaws.com", "sts.us-east-1.amazonaws.com"],
        kind: "emulated",
        port: 443,
        to: "sts-shim",
      },
      {
        from: "envoy",
        hosts: [
          "s3.amazonaws.com",
          "s3.us-east-1.amazonaws.com",
          "unity.s3.amazonaws.com",
          "mlflow.s3.amazonaws.com",
          "unity.s3.us-east-1.amazonaws.com",
          "mlflow.s3.us-east-1.amazonaws.com",
        ],
        kind: "emulated",
        port: 443,
        to: "rustfs",
      },
      {
        from: "@host",
        host_ports: [9080, 9100, 9901],
        kind: "ingress",
        to: "envoy",
      },
    ],
    nodes: [
      {
        category: "gateway",
        display_name: "Envoy gateway",
        id: "envoy",
        placement: "container:envoy",
        role: "gateway",
        summary: "Single-port gateway, Databricks-shaped URL rewrites.",
      },
      {
        category: "observability",
        display_name: "Jaeger tracing",
        id: "jaeger",
        placement: "container:jaeger",
        role: "tracing",
        summary: "All-in-one OTLP tracing backend with the Jaeger UI.",
      },
      {
        category: "metadata_db",
        display_name: "Postgres",
        id: "postgres",
        placement: "container:db",
        role: "relational_db",
        summary: "Postgres 16; auto-creates DBs other modules declare.",
      },
      {
        category: "storage",
        display_name: "RustFS (local S3 + STS)",
        id: "rustfs",
        placement: "container:rustfs",
        role: "object_store",
        summary:
          "Self-hosted S3-compatible object store with STS; answers the AWS S3/STS hostnames in-network.",
      },
      {
        category: "catalog",
        display_name: "Unity Catalog",
        id: "unity-catalog",
        placement: "container:unitycatalog",
        role: "data_catalog",
        summary: "Databricks UC server; Databricks-shaped REST API.",
      },
      {
        category: "ml",
        display_name: "MLflow tracking",
        id: "mlflow",
        placement: "container:mlflow",
        role: "experiment_tracking",
        summary: "Experiment + model tracking; Databricks-shaped URLs.",
      },
    ],
    services: [
      {
        exposed: [],
        id: "@host",
        image: null,
        kind: "host",
        module: null,
        published: [],
        role: null,
      },
      {
        exposed: [],
        id: "envoy",
        image: "envoyproxy/envoy:v1.34-latest",
        kind: "container",
        module: "envoy",
        published: [
          {
            container: 10000,
            host: 9080,
          },
          {
            container: 9100,
            host: 9100,
          },
          {
            container: 9901,
            host: 9901,
          },
        ],
        role: "gateway",
      },
      {
        exposed: [16686, 4317],
        id: "jaeger",
        image: "cr.jaegertracing.io/jaegertracing/jaeger:2.14.1",
        kind: "container",
        module: "jaeger",
        published: [],
        role: "tracing",
      },
      {
        exposed: [5432],
        id: "db",
        image: "postgres:16",
        kind: "container",
        module: "postgres",
        published: [],
        role: "relational_db",
      },
      {
        exposed: [8081],
        id: "pgweb",
        image: "sosedoff/pgweb:latest",
        kind: "container",
        module: "postgres",
        published: [],
        role: null,
      },
      {
        exposed: [9000],
        id: "rustfs",
        image: "rustfs/rustfs:1.0.1",
        kind: "container",
        module: "rustfs",
        published: [],
        role: "object_store",
      },
      {
        exposed: [],
        id: "sts-shim",
        image: "python:3.13-alpine",
        kind: "container",
        module: "rustfs",
        published: [],
        role: null,
      },
      {
        exposed: [8080],
        id: "unitycatalog",
        image: "unitycatalog/unitycatalog:v0.6.0",
        kind: "container",
        module: "unity-catalog",
        published: [],
        role: "data_catalog",
      },
      {
        exposed: [5000],
        id: "mlflow",
        image: "ghcr.io/mlflow/mlflow:v3.10.1-full",
        kind: "container",
        module: "mlflow",
        published: [],
        role: "experiment_tracking",
      },
    ],
  },
  services: {
    envoy: [
      {
        depends_on: [],
        endpoints: [
          {
            host_port: 9080,
            id: "http",
            intent: {
              kind: "internal",
            },
            internal_port: 10000,
            rewrite: "inherit",
            scheme: "http",
          },
        ],
        name: "envoy",
        placement: {
          kind: "container",
          service: "envoy",
        },
        role: "gateway",
      },
    ],
    jaeger: [
      {
        base_path: "/jaeger",
        depends_on: [],
        endpoints: [
          {
            host_port: 16686,
            id: "ui",
            intent: {
              kind: "ui_prefixable",
            },
            internal_port: 16686,
            rewrite: "inherit",
            scheme: "http",
          },
          {
            host_port: 4317,
            id: "otlp_grpc",
            intent: {
              kind: "internal",
            },
            internal_port: 4317,
            rewrite: "inherit",
            scheme: "grpc",
          },
        ],
        name: "jaeger",
        placement: {
          kind: "container",
          service: "jaeger",
        },
        role: "tracing",
      },
    ],
    mlflow: [
      {
        base_path: "/mlflow",
        depends_on: [],
        endpoints: [
          {
            id: "tracking",
            intent: {
              kind: "api",
            },
            internal_port: 5000,
            mount_prefix: "/api/2.0/mlflow",
            rewrite: "inherit",
            scheme: "http",
          },
          {
            id: "otel",
            intent: {
              kind: "api",
            },
            internal_port: 5000,
            mount_prefix: "/api/2.0/otel",
            rewrite: "passthrough",
            scheme: "http",
          },
          {
            id: "ui",
            intent: {
              kind: "ui_prefixable",
            },
            internal_port: 5000,
            rewrite: "inherit",
            scheme: "http",
          },
        ],
        name: "mlflow",
        placement: {
          kind: "container",
          service: "mlflow",
        },
        role: "experiment_tracking",
      },
    ],
    postgres: [
      {
        depends_on: [],
        endpoints: [
          {
            host_port: 5432,
            id: "sql",
            intent: {
              kind: "internal",
            },
            internal_port: 5432,
            rewrite: "inherit",
            scheme: "tcp",
          },
        ],
        name: "db",
        placement: {
          kind: "container",
          service: "db",
        },
        role: "relational_db",
      },
    ],
    rustfs: [
      {
        depends_on: [],
        endpoints: [
          {
            id: "s3",
            intent: {
              kind: "gatewayed",
            },
            internal_port: 9000,
            rewrite: "inherit",
            scheme: "http",
          },
        ],
        name: "rustfs",
        placement: {
          kind: "container",
          service: "rustfs",
        },
        role: "object_store",
      },
    ],
    "unity-catalog": [
      {
        depends_on: [],
        endpoints: [
          {
            id: "rest",
            intent: {
              kind: "api",
            },
            internal_port: 8080,
            mount_prefix: "/api/2.1/unity-catalog",
            rewrite: "inherit",
            scheme: "http",
          },
          {
            id: "rest_alias",
            intent: {
              kind: "api",
            },
            internal_port: 8080,
            mount_prefix: "/unity-catalog",
            rewrite: "inherit",
            scheme: "http",
          },
        ],
        name: "unitycatalog",
        placement: {
          kind: "container",
          service: "unitycatalog",
        },
        role: "data_catalog",
      },
    ],
  },
  gateway: {
    admin_port: 9901,
    auth: null,
    clusters: [
      {
        host: "jaeger",
        name: "jaeger",
        port: 16686,
      },
      {
        host: "mlflow",
        name: "mlflow",
        port: 5000,
      },
      {
        host: "rustfs",
        name: "rustfs",
        port: 9000,
      },
      {
        host: "sts-shim",
        name: "sts-shim",
        port: 8080,
      },
      {
        host: "unitycatalog",
        name: "unitycatalog",
        port: 8080,
      },
    ],
    listeners: [
      {
        host_port: 9080,
        routes: [
          {
            cluster: "unitycatalog",
            prefix: "/api/2.1/unity-catalog",
            rewrite: null,
          },
          {
            cluster: "mlflow",
            prefix: "/api/2.0/mlflow",
            rewrite: "/mlflow/api/2.0/mlflow",
          },
          {
            cluster: "unitycatalog",
            prefix: "/unity-catalog",
            rewrite: null,
          },
          {
            cluster: "mlflow",
            prefix: "/api/2.0/otel",
            rewrite: null,
          },
          {
            cluster: "jaeger",
            prefix: "/jaeger",
            rewrite: null,
          },
          {
            cluster: "mlflow",
            prefix: "/mlflow",
            rewrite: null,
          },
        ],
      },
      {
        host_port: 9100,
        routes: [
          {
            cluster: "rustfs",
            prefix: "/",
            rewrite: null,
          },
        ],
      },
    ],
    tls: {
      hosts: [
        "mlflow.s3.amazonaws.com",
        "mlflow.s3.us-east-1.amazonaws.com",
        "s3.amazonaws.com",
        "s3.us-east-1.amazonaws.com",
        "sts.amazonaws.com",
        "sts.us-east-1.amazonaws.com",
        "unity.s3.amazonaws.com",
        "unity.s3.us-east-1.amazonaws.com",
      ],
      port: 443,
      virtual_hosts: [
        {
          cluster: "sts-shim",
          hosts: ["sts.amazonaws.com", "sts.us-east-1.amazonaws.com"],
        },
        {
          cluster: "rustfs",
          hosts: [
            "s3.amazonaws.com",
            "s3.us-east-1.amazonaws.com",
            "unity.s3.amazonaws.com",
            "mlflow.s3.amazonaws.com",
            "unity.s3.us-east-1.amazonaws.com",
            "mlflow.s3.us-east-1.amazonaws.com",
          ],
        },
      ],
    },
  },
  files: FIXTURE_FILES,
};
