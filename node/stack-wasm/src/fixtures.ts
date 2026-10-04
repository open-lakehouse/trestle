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
        id: "@clients",
        kind: "clients",
        display_name: "Clients",
        summary: "Browsers, SDKs and CLIs on the host.",
        category: null,
        role: null,
        placement: null,
        exposes: [],
        offers: [],
        provisions: [],
      },
      {
        id: "envoy",
        kind: "gateway",
        display_name: "Envoy gateway",
        summary: "Single-port gateway, Databricks-shaped URL rewrites.",
        category: "gateway",
        role: "gateway",
        placement: "container:envoy",
        exposes: [
          {
            module: "jaeger",
            endpoint: "ui",
            kind: "ui",
            prefix: "/jaeger",
            host_port: 9080,
            gated: false,
          },
          {
            module: "rustfs",
            endpoint: "s3",
            kind: "service",
            prefix: "/",
            host_port: 9100,
            gated: false,
          },
          {
            module: "unity-catalog",
            endpoint: "rest",
            kind: "api",
            prefix: "/api/2.1/unity-catalog",
            host_port: 9080,
            gated: false,
          },
          {
            module: "unity-catalog",
            endpoint: "rest_alias",
            kind: "api",
            prefix: "/unity-catalog",
            host_port: 9080,
            gated: false,
          },
          {
            module: "mlflow",
            endpoint: "tracking",
            kind: "api",
            prefix: "/api/2.0/mlflow",
            host_port: 9080,
            gated: false,
          },
          {
            module: "mlflow",
            endpoint: "otel",
            kind: "api",
            prefix: "/api/2.0/otel",
            host_port: 9080,
            gated: false,
          },
          {
            module: "mlflow",
            endpoint: "ui",
            kind: "ui",
            prefix: "/mlflow",
            host_port: 9080,
            gated: false,
          },
        ],
        offers: [],
        provisions: [],
      },
      {
        id: "jaeger",
        kind: "component",
        display_name: "Jaeger tracing",
        summary: "All-in-one OTLP tracing backend with the Jaeger UI.",
        category: "observability",
        role: "tracing",
        placement: "container:jaeger",
        exposes: [],
        offers: [],
        provisions: [],
      },
      {
        id: "postgres",
        kind: "component",
        display_name: "Postgres",
        summary: "Postgres 16; auto-creates DBs other modules declare.",
        category: "metadata_db",
        role: "relational_db",
        placement: "container:db",
        exposes: [],
        offers: ["PostgreSQL"],
        provisions: [
          {
            resource: "relational_db",
            name: "unitycatalog",
          },
          {
            resource: "relational_db",
            name: "mlflow",
          },
        ],
      },
      {
        id: "rustfs",
        kind: "component",
        display_name: "RustFS (local S3 + STS)",
        summary:
          "Self-hosted S3-compatible object store with STS; answers the AWS S3/STS hostnames in-network.",
        category: "storage",
        role: "object_store",
        placement: "container:rustfs",
        exposes: [],
        offers: ["S3 + STS"],
        provisions: [
          {
            resource: "object_store",
            name: "unity",
          },
          {
            resource: "object_store",
            name: "mlflow",
          },
        ],
      },
      {
        id: "unity-catalog",
        kind: "component",
        display_name: "Unity Catalog",
        summary: "Databricks UC server; Databricks-shaped REST API.",
        category: "catalog",
        role: "data_catalog",
        placement: "container:unitycatalog",
        exposes: [],
        offers: [],
        provisions: [],
      },
      {
        id: "mlflow",
        kind: "component",
        display_name: "MLflow tracking",
        summary: "Experiment + model tracking; Databricks-shaped URLs.",
        category: "ml",
        role: "experiment_tracking",
        placement: "container:mlflow",
        exposes: [],
        offers: [],
        provisions: [],
      },
    ],
    edges: [
      {
        kind: "access",
        from: "@clients",
        to: "envoy",
        host_ports: [9080, 9100],
      },
      {
        kind: "route",
        from: "envoy",
        to: "jaeger",
        gated: false,
      },
      {
        kind: "route",
        from: "envoy",
        to: "rustfs",
        gated: false,
      },
      {
        kind: "route",
        from: "envoy",
        to: "unity-catalog",
        gated: false,
      },
      {
        kind: "route",
        from: "envoy",
        to: "mlflow",
        gated: false,
      },
      {
        kind: "uses",
        from: "unity-catalog",
        to: "postgres",
        role: "relational_db",
        protocol: "PostgreSQL",
        resources: ["unitycatalog"],
      },
      {
        kind: "uses",
        from: "unity-catalog",
        to: "rustfs",
        role: "object_store",
        protocol: "S3",
        resources: ["unity"],
      },
      {
        kind: "uses",
        from: "mlflow",
        to: "postgres",
        role: "relational_db",
        protocol: "PostgreSQL",
        resources: ["mlflow"],
      },
      {
        kind: "uses",
        from: "mlflow",
        to: "rustfs",
        role: "object_store",
        protocol: "S3",
        resources: ["mlflow"],
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
