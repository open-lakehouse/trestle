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
    nodes: [
      {
        id: "envoy",
        display_name: "Envoy gateway",
        summary: "Single-port gateway, Databricks-shaped URL rewrites.",
        category: "gateway",
        role: "gateway",
        placement: "container:envoy",
      },
      {
        id: "jaeger",
        display_name: "Jaeger tracing",
        summary: "All-in-one OTLP tracing backend with the Jaeger UI.",
        category: "observability",
        role: "tracing",
        placement: "container:jaeger",
      },
      {
        id: "postgres",
        display_name: "Postgres",
        summary: "Postgres 16; auto-creates DBs other modules declare.",
        category: "metadata_db",
        role: "relational_db",
        placement: "container:db",
      },
      {
        id: "rustfs",
        display_name: "RustFS (local S3 + STS)",
        summary:
          "Self-hosted S3-compatible object store with STS; answers the AWS S3/STS hostnames in-network.",
        category: "storage",
        role: "object_store",
        placement: "container:rustfs",
      },
      {
        id: "unity-catalog",
        display_name: "Unity Catalog",
        summary: "Databricks UC server; Databricks-shaped REST API.",
        category: "catalog",
        role: "data_catalog",
        placement: "container:unitycatalog",
      },
      {
        id: "mlflow",
        display_name: "MLflow tracking",
        summary: "Experiment + model tracking; Databricks-shaped URLs.",
        category: "ml",
        role: "experiment_tracking",
        placement: "container:mlflow",
      },
    ],
    services: [
      {
        id: "@host",
        module: null,
        kind: "host",
        role: null,
        image: null,
        published: [],
        exposed: [],
      },
      {
        id: "envoy",
        module: "envoy",
        kind: "container",
        role: "gateway",
        image: "envoyproxy/envoy:v1.34-latest",
        published: [
          {
            host: 9080,
            container: 10000,
          },
          {
            host: 9100,
            container: 9100,
          },
          {
            host: 9901,
            container: 9901,
          },
        ],
        exposed: [],
      },
      {
        id: "jaeger",
        module: "jaeger",
        kind: "container",
        role: "tracing",
        image: "cr.jaegertracing.io/jaegertracing/jaeger:2.14.1",
        published: [],
        exposed: [16686, 4317],
      },
      {
        id: "db",
        module: "postgres",
        kind: "container",
        role: "relational_db",
        image: "postgres:16",
        published: [],
        exposed: [5432],
      },
      {
        id: "rustfs",
        module: "rustfs",
        kind: "container",
        role: "object_store",
        image: "rustfs/rustfs:1.0.1",
        published: [],
        exposed: [9000],
      },
      {
        id: "sts-shim",
        module: "rustfs",
        kind: "container",
        role: null,
        image: "python:3.13-alpine",
        published: [],
        exposed: [],
      },
      {
        id: "unitycatalog",
        module: "unity-catalog",
        kind: "container",
        role: "data_catalog",
        image: "unitycatalog/unitycatalog:v0.6.0",
        published: [],
        exposed: [8080],
      },
      {
        id: "mlflow",
        module: "mlflow",
        kind: "container",
        role: "experiment_tracking",
        image: "ghcr.io/mlflow/mlflow:v3.10.1-full",
        published: [],
        exposed: [5000],
      },
    ],
    edges: [
      {
        kind: "startup",
        from: "mlflow",
        to: "db",
        condition: "service_healthy",
      },
      {
        kind: "startup",
        from: "mlflow",
        to: "rustfs",
        condition: "service_healthy",
      },
      {
        kind: "startup",
        from: "sts-shim",
        to: "rustfs",
        condition: "service_healthy",
      },
      {
        kind: "startup",
        from: "unitycatalog",
        to: "db",
        condition: "service_healthy",
      },
      {
        kind: "startup",
        from: "unitycatalog",
        to: "rustfs",
        condition: "service_healthy",
      },
      {
        kind: "route",
        from: "envoy",
        to: "unitycatalog",
        routes: [
          {
            prefix: "/api/2.1/unity-catalog",
            host_port: 9080,
            rewrite: null,
            gated: false,
          },
          {
            prefix: "/unity-catalog",
            host_port: 9080,
            rewrite: null,
            gated: false,
          },
        ],
      },
      {
        kind: "route",
        from: "envoy",
        to: "mlflow",
        routes: [
          {
            prefix: "/api/2.0/mlflow",
            host_port: 9080,
            rewrite: "/mlflow/api/2.0/mlflow",
            gated: false,
          },
          {
            prefix: "/api/2.0/otel",
            host_port: 9080,
            rewrite: null,
            gated: false,
          },
          {
            prefix: "/mlflow",
            host_port: 9080,
            rewrite: null,
            gated: false,
          },
        ],
      },
      {
        kind: "route",
        from: "envoy",
        to: "jaeger",
        routes: [
          {
            prefix: "/jaeger",
            host_port: 9080,
            rewrite: null,
            gated: false,
          },
        ],
      },
      {
        kind: "route",
        from: "envoy",
        to: "rustfs",
        routes: [
          {
            prefix: "/",
            host_port: 9100,
            rewrite: null,
            gated: false,
          },
        ],
      },
      {
        kind: "emulated",
        from: "unitycatalog",
        to: "envoy",
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
      },
      {
        kind: "emulated",
        from: "envoy",
        to: "sts-shim",
        hosts: ["sts.amazonaws.com", "sts.us-east-1.amazonaws.com"],
        port: 443,
      },
      {
        kind: "emulated",
        from: "envoy",
        to: "rustfs",
        hosts: [
          "s3.amazonaws.com",
          "s3.us-east-1.amazonaws.com",
          "unity.s3.amazonaws.com",
          "mlflow.s3.amazonaws.com",
          "unity.s3.us-east-1.amazonaws.com",
          "mlflow.s3.us-east-1.amazonaws.com",
        ],
        port: 443,
      },
      {
        kind: "ingress",
        from: "@host",
        to: "envoy",
        host_ports: [9080, 9100, 9901],
      },
    ],
  },
  services: {
    envoy: [
      {
        name: "envoy",
        role: "gateway",
        placement: {
          kind: "container",
          service: "envoy",
        },
        endpoints: [
          {
            id: "http",
            scheme: "http",
            internal_port: 10000,
            host_port: 9080,
            intent: {
              kind: "internal",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
      },
    ],
    jaeger: [
      {
        name: "jaeger",
        role: "tracing",
        placement: {
          kind: "container",
          service: "jaeger",
        },
        endpoints: [
          {
            id: "ui",
            scheme: "http",
            internal_port: 16686,
            host_port: 16686,
            intent: {
              kind: "ui_prefixable",
            },
            rewrite: "inherit",
          },
          {
            id: "otlp_grpc",
            scheme: "grpc",
            internal_port: 4317,
            host_port: 4317,
            intent: {
              kind: "internal",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
        base_path: "/jaeger",
      },
    ],
    mlflow: [
      {
        name: "mlflow",
        role: "experiment_tracking",
        placement: {
          kind: "container",
          service: "mlflow",
        },
        endpoints: [
          {
            id: "tracking",
            scheme: "http",
            internal_port: 5000,
            intent: {
              kind: "api",
            },
            mount_prefix: "/api/2.0/mlflow",
            rewrite: "inherit",
          },
          {
            id: "otel",
            scheme: "http",
            internal_port: 5000,
            intent: {
              kind: "api",
            },
            mount_prefix: "/api/2.0/otel",
            rewrite: "passthrough",
          },
          {
            id: "ui",
            scheme: "http",
            internal_port: 5000,
            intent: {
              kind: "ui_prefixable",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
        base_path: "/mlflow",
      },
    ],
    postgres: [
      {
        name: "db",
        role: "relational_db",
        placement: {
          kind: "container",
          service: "db",
        },
        endpoints: [
          {
            id: "sql",
            scheme: "tcp",
            internal_port: 5432,
            host_port: 5432,
            intent: {
              kind: "internal",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
      },
    ],
    rustfs: [
      {
        name: "rustfs",
        role: "object_store",
        placement: {
          kind: "container",
          service: "rustfs",
        },
        endpoints: [
          {
            id: "s3",
            scheme: "http",
            internal_port: 9000,
            intent: {
              kind: "gatewayed",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
      },
    ],
    "unity-catalog": [
      {
        name: "unitycatalog",
        role: "data_catalog",
        placement: {
          kind: "container",
          service: "unitycatalog",
        },
        endpoints: [
          {
            id: "rest",
            scheme: "http",
            internal_port: 8080,
            intent: {
              kind: "api",
            },
            mount_prefix: "/api/2.1/unity-catalog",
            rewrite: "inherit",
          },
          {
            id: "rest_alias",
            scheme: "http",
            internal_port: 8080,
            intent: {
              kind: "api",
            },
            mount_prefix: "/unity-catalog",
            rewrite: "inherit",
          },
        ],
        depends_on: [],
      },
    ],
  },
  gateway: {
    listeners: [
      {
        host_port: 9080,
        routes: [
          {
            prefix: "/api/2.1/unity-catalog",
            cluster: "unitycatalog",
            rewrite: null,
          },
          {
            prefix: "/api/2.0/mlflow",
            cluster: "mlflow",
            rewrite: "/mlflow/api/2.0/mlflow",
          },
          {
            prefix: "/unity-catalog",
            cluster: "unitycatalog",
            rewrite: null,
          },
          {
            prefix: "/api/2.0/otel",
            cluster: "mlflow",
            rewrite: null,
          },
          {
            prefix: "/jaeger",
            cluster: "jaeger",
            rewrite: null,
          },
          {
            prefix: "/mlflow",
            cluster: "mlflow",
            rewrite: null,
          },
        ],
      },
      {
        host_port: 9100,
        routes: [
          {
            prefix: "/",
            cluster: "rustfs",
            rewrite: null,
          },
        ],
      },
    ],
    clusters: [
      {
        name: "jaeger",
        host: "jaeger",
        port: 16686,
      },
      {
        name: "mlflow",
        host: "mlflow",
        port: 5000,
      },
      {
        name: "rustfs",
        host: "rustfs",
        port: 9000,
      },
      {
        name: "sts-shim",
        host: "sts-shim",
        port: 8080,
      },
      {
        name: "unitycatalog",
        host: "unitycatalog",
        port: 8080,
      },
    ],
    admin_port: 9901,
    auth: null,
    tls: {
      port: 443,
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
