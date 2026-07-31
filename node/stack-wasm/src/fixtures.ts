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
      id: "seaweedfs",
      display_name: "SeaweedFS (local S3)",
      summary: "Self-hosted S3-compatible object store.",
      category: "storage",
      provider_of: "object_store",
      requires: [],
      conflicts_with: [],
      knobs: [
        {
          key: "image",
          title: "SeaweedFS image",
          kind: {
            kind: "string",
          },
          default: "chrislusf/seaweedfs:latest",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["SEAWEEDFS_IMAGE"],
        },
        {
          key: "init_image",
          title: "SeaweedFS init image",
          kind: {
            kind: "string",
          },
          default: "amazon/aws-cli:latest",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["SEAWEEDFS_INIT_IMAGE"],
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
          default: "unitycatalog/unitycatalog:main-2f2e32d",
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
    modules: ["envoy", "postgres", "seaweedfs"],
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
        id: "seaweedfs",
        display_name: "SeaweedFS (local S3)",
        summary: "Self-hosted S3-compatible object store.",
        category: "storage",
        role: "object_store",
        placement: "container:seaweedfs",
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
    edges: [
      {
        from: "jaeger",
        to: "envoy",
      },
      {
        from: "mlflow",
        to: "envoy",
      },
      {
        from: "mlflow",
        to: "postgres",
      },
      {
        from: "mlflow",
        to: "seaweedfs",
      },
      {
        from: "unity-catalog",
        to: "envoy",
      },
      {
        from: "unity-catalog",
        to: "postgres",
      },
      {
        from: "unity-catalog",
        to: "seaweedfs",
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
    seaweedfs: [
      {
        name: "seaweedfs",
        role: "object_store",
        placement: {
          kind: "container",
          service: "seaweedfs",
        },
        endpoints: [
          {
            id: "s3",
            scheme: "http",
            internal_port: 8333,
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
            cluster: "seaweedfs",
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
        name: "seaweedfs",
        host: "seaweedfs",
        port: 8333,
      },
      {
        name: "unitycatalog",
        host: "unitycatalog",
        port: 8080,
      },
    ],
  },
  files: FIXTURE_FILES,
};
