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
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
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
          advanced: true,
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
          advanced: true,
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
      resource_kinds: [],
      resource_nouns: [],
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
          advanced: true,
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
      resource_kinds: ["relational_db"],
      resource_nouns: [
        {
          kind: "relational_db",
          singular: "database",
          plural: "databases",
        },
      ],
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
          advanced: true,
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
      resource_kinds: ["object_store"],
      resource_nouns: [
        {
          kind: "object_store",
          singular: "bucket",
          plural: "buckets",
        },
      ],
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
          advanced: true,
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
          advanced: true,
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
          advanced: true,
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
      resource_kinds: ["object_store"],
      resource_nouns: [
        {
          kind: "object_store",
          singular: "container",
          plural: "containers",
        },
      ],
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
          advanced: true,
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
          advanced: true,
        },
      ],
    },
    {
      id: "mlflow",
      display_name: "MLflow tracking",
      summary:
        "Experiment + model tracking and an AI Gateway; Databricks-shaped URLs.",
      category: "ml",
      provider_of: "experiment_tracking",
      requires: ["envoy"],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
        {
          key: "image",
          title: "MLflow image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/mlflow/mlflow:v3.17.0-full",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["MLFLOW_IMAGE"],
          advanced: true,
        },
        {
          key: "ai_gateway_models",
          title: "AI Gateway models",
          kind: {
            kind: "string",
          },
          default:
            "anthropic:claude-opus-5-5,anthropic:claude-sonnet-5-5,anthropic:claude-haiku-5-5",
          required: false,
          help: "Comma-separated provider:model pairs to seed as AI Gateway endpoints, each named the model id. A pair is seeded only when its provider key (e.g. ANTHROPIC_API_KEY) is exported before `docker compose up`.",
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
      resource_kinds: [],
      resource_nouns: [],
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
          advanced: true,
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
      resource_kinds: [],
      resource_nouns: [],
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
          advanced: true,
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
      resource_kinds: [],
      resource_nouns: [],
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
          advanced: true,
        },
      ],
    },
    {
      id: "omnigent",
      display_name: "Omnigent",
      summary:
        "AI-agent platform; agents run in the stack, models via MLflow's AI Gateway.",
      category: "agents",
      provider_of: "agent_platform",
      requires: ["envoy", "mlflow"],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
        {
          key: "host",
          title: "Run an agent host in the stack",
          kind: {
            kind: "bool",
          },
          default: "true",
          required: false,
          help: "Run agents in a host container on the stack's network, with the lakehouse's client env and models from MLflow's AI Gateway. Off: attach your own host with `omnigent host $OMNIGENT_URL`. Always off while the gateway authenticates.",
        },
        {
          key: "model",
          title: "Agent model",
          kind: {
            kind: "string",
          },
          default: "claude-opus-5-5",
          required: false,
          help: "The model agents use by default: an AI Gateway endpoint name (see mlflow.ai_gateway_models).",
        },
        {
          key: "fast_model",
          title: "Background model",
          kind: {
            kind: "string",
          },
          default: "claude-haiku-5-5",
          required: false,
          help: "The model harnesses use for quick background calls: an AI Gateway endpoint name.",
        },
        {
          key: "image",
          title: "Omnigent server image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/omnigent-ai/omnigent-server:v0.17.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["OMNIGENT_IMAGE"],
          advanced: true,
        },
        {
          key: "host_image",
          title: "Omnigent host image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/omnigent-ai/omnigent-host:v0.17.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["OMNIGENT_HOST_IMAGE"],
          advanced: true,
        },
      ],
    },
    {
      id: "marimo",
      display_name: "marimo notebooks",
      summary: "Hosted marimo editor for the client workspace's notebooks.",
      category: "notebooks",
      provider_of: "notebooks",
      requires: ["envoy"],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
        {
          key: "image",
          title: "marimo runtime image",
          kind: {
            kind: "string",
          },
          default: "ghcr.io/astral-sh/uv:0.13.0-python3.12-trixie-slim",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["MARIMO_IMAGE"],
          advanced: true,
        },
      ],
    },
    {
      id: "puppygraph",
      display_name: "PuppyGraph",
      summary: "Graph queries (Cypher, Gremlin) over Unity Catalog tables.",
      category: "graph",
      provider_of: "graph",
      requires: ["envoy", "unity-catalog"],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
        {
          key: "catalog",
          title: "Unity Catalog catalog",
          kind: {
            kind: "string",
          },
          default: "unity",
          required: false,
          help: "The UC catalog whose schemas and tables the graph schema maps.",
        },
        {
          key: "uc_url",
          title: "Unity Catalog URL",
          kind: {
            kind: "string",
          },
          default: "http://unitycatalog:8080",
          required: false,
          help: "Where PuppyGraph reaches Unity Catalog's REST API, directly on the compose network (not through the gateway).",
          advanced: true,
        },
        {
          key: "password",
          title: "PuppyGraph password",
          kind: {
            kind: "string",
          },
          default: "puppygraph123",
          required: false,
          help: "Password of the `puppygraph` user for the web UI, Bolt and Gremlin. A local default; set your own if the ports are reachable by others.",
          advanced: true,
        },
        {
          key: "image",
          title: "PuppyGraph image",
          kind: {
            kind: "string",
          },
          default: "puppygraph/puppygraph:1.14.0",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["PUPPYGRAPH_IMAGE"],
          advanced: true,
        },
      ],
    },
    {
      id: "opa",
      display_name: "Open Policy Agent",
      summary: "Rego authorization for every gateway request, after sign-on.",
      category: "gateway",
      provider_of: "policy",
      requires: [],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [
        {
          key: "image",
          title: "OPA image",
          kind: {
            kind: "string",
          },
          default: "openpolicyagent/opa:1.21.1-envoy-static",
          required: false,
          help: "Container image reference (repository:tag or digest). Override to pin or test a different release.",
          aliases: ["OPA_IMAGE"],
          advanced: true,
        },
        {
          key: "dry_run",
          title: "Dry run",
          kind: {
            kind: "bool",
          },
          default: "false",
          required: false,
          help: "Allow every request and only log the policy's decision — for rolling out a policy change safely.",
          advanced: true,
        },
        {
          key: "log_level",
          title: "Log level",
          kind: {
            kind: "enum",
            options: ["debug", "info", "warn", "error"],
          },
          default: "info",
          required: false,
          advanced: true,
        },
      ],
    },
    {
      id: "databricks-emulator-env",
      display_name: "Databricks app runtime contract",
      summary:
        "DATABRICKS_HOST / app URL / forwarded-user env vars apps expect.",
      category: "app_runtime",
      provider_of: "databricks_apps_contract",
      requires: [],
      conflicts_with: [],
      resource_kinds: [],
      resource_nouns: [],
      knobs: [],
    },
  ],
  default_selection: {
    modules: ["postgres", "rustfs", "unity-catalog"],
    capabilities: [],
    knob_overrides: {},
    extra_resources: [],
    gateway: {
      auth: false,
      policy: false,
    },
  },
  targets: [
    {
      id: "compose",
      display_name: "Docker Compose",
    },
  ],
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
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "authelia",
        kind: "component",
        display_name: "Authelia",
        summary: "Forward-auth single-sign-on for the gateway (file-based).",
        category: "gateway",
        role: "auth",
        placement: "container:authelia",
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "envoy",
        kind: "gateway",
        display_name: "Envoy gateway",
        summary: "Single-port gateway, Databricks-shaped URL rewrites.",
        category: "gateway",
        role: "gateway",
        placement: "container:envoy",
        cloud: null,
        surfaces: [
          {
            host_port: 9080,
            kind: "platform",
            protocols: [],
            exposes: [
              {
                module: "authelia",
                endpoint: "portal",
                kind: "ui",
                prefix: "/authelia",
                gated: false,
              },
              {
                module: "jaeger",
                endpoint: "ui",
                kind: "ui",
                prefix: "/jaeger",
                gated: true,
              },
              {
                module: "jaeger",
                endpoint: "otlp_http",
                kind: "api",
                prefix: "/v1/traces",
                gated: true,
              },
              {
                module: "unity-catalog",
                endpoint: "rest",
                kind: "api",
                prefix: "/api/2.1/unity-catalog",
                gated: true,
              },
              {
                module: "unity-catalog",
                endpoint: "rest_alias",
                kind: "api",
                prefix: "/unity-catalog",
                gated: true,
              },
              {
                module: "mlflow",
                endpoint: "tracking",
                kind: "api",
                prefix: "/api/2.0/mlflow",
                gated: true,
              },
              {
                module: "mlflow",
                endpoint: "otel",
                kind: "api",
                prefix: "/api/2.0/otel",
                gated: true,
              },
              {
                module: "mlflow",
                endpoint: "ai_gateway",
                kind: "api",
                prefix: "/ai-gateway",
                gated: true,
              },
              {
                module: "mlflow",
                endpoint: "ui",
                kind: "ui",
                prefix: "/mlflow",
                gated: true,
              },
            ],
          },
          {
            host_port: 9100,
            kind: "service",
            protocols: ["S3 + STS"],
            exposes: [
              {
                module: "rustfs",
                endpoint: "s3",
                kind: "service",
                prefix: "/",
                gated: false,
              },
            ],
          },
        ],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "jaeger",
        kind: "component",
        display_name: "Jaeger tracing",
        summary: "All-in-one OTLP tracing backend with the Jaeger UI.",
        category: "observability",
        role: "tracing",
        placement: "container:jaeger",
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "opa",
        kind: "component",
        display_name: "Open Policy Agent",
        summary: "Rego authorization for every gateway request, after sign-on.",
        category: "gateway",
        role: "policy",
        placement: "container:opa",
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "postgres",
        kind: "component",
        display_name: "Postgres",
        summary: "Postgres 16; auto-creates DBs other modules declare.",
        category: "metadata_db",
        role: "relational_db",
        placement: "container:db",
        cloud: null,
        surfaces: [],
        offers: ["PostgreSQL"],
        provisions: [
          {
            resource: "relational_db",
            name: "unitycatalog",
            consumers: ["unity-catalog"],
          },
          {
            resource: "relational_db",
            name: "mlflow",
            consumers: ["mlflow"],
          },
        ],
        resource_nouns: [
          {
            kind: "relational_db",
            singular: "database",
            plural: "databases",
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
        cloud: null,
        surfaces: [],
        offers: ["S3 + STS"],
        provisions: [
          {
            resource: "object_store",
            name: "unity",
            consumers: ["unity-catalog"],
          },
          {
            resource: "object_store",
            name: "mlflow",
            consumers: ["mlflow"],
          },
        ],
        resource_nouns: [
          {
            kind: "object_store",
            singular: "bucket",
            plural: "buckets",
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
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
      {
        id: "mlflow",
        kind: "component",
        display_name: "MLflow tracking",
        summary:
          "Experiment + model tracking and an AI Gateway; Databricks-shaped URLs.",
        category: "ml",
        role: "experiment_tracking",
        placement: "container:mlflow",
        cloud: null,
        surfaces: [],
        offers: [],
        provisions: [],
        resource_nouns: [],
      },
    ],
    edges: [
      {
        kind: "access",
        from: "@clients",
        to: "envoy",
        host_port: 9080,
      },
      {
        kind: "access",
        from: "@clients",
        to: "envoy",
        host_port: 9100,
      },
      {
        kind: "route",
        from: "envoy",
        to: "jaeger",
        host_port: 9080,
        gated: true,
      },
      {
        kind: "route",
        from: "envoy",
        to: "unity-catalog",
        host_port: 9080,
        gated: true,
      },
      {
        kind: "route",
        from: "envoy",
        to: "mlflow",
        host_port: 9080,
        gated: true,
      },
      {
        kind: "route",
        from: "envoy",
        to: "rustfs",
        host_port: 9100,
        gated: false,
      },
      {
        kind: "authenticates",
        from: "envoy",
        to: "authelia",
        host_port: 9080,
        portal_prefix: "/authelia",
      },
      {
        kind: "authorizes",
        from: "envoy",
        to: "opa",
        host_port: 9080,
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
    authelia: [
      {
        name: "authelia",
        role: "auth",
        placement: {
          kind: "container",
          service: "authelia",
        },
        endpoints: [
          {
            id: "http",
            scheme: "http",
            internal_port: 9091,
            intent: {
              kind: "internal",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
      },
    ],
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
          {
            id: "otlp_http",
            scheme: "http",
            internal_port: 4318,
            intent: {
              kind: "api",
            },
            mount_prefix: "/v1/traces",
            rewrite: "passthrough",
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
            rewrite: {
              to: "/mlflow",
            },
          },
          {
            id: "ai_gateway",
            scheme: "http",
            internal_port: 5000,
            intent: {
              kind: "api",
            },
            mount_prefix: "/ai-gateway",
            rewrite: {
              to: "/mlflow/gateway",
            },
            route: {
              streaming: true,
              strip_client_credentials: true,
            },
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
    opa: [
      {
        name: "opa",
        role: "policy",
        placement: {
          kind: "container",
          service: "opa",
        },
        endpoints: [
          {
            id: "grpc",
            scheme: "grpc",
            internal_port: 9191,
            intent: {
              kind: "internal",
            },
            rewrite: "inherit",
          },
        ],
        depends_on: [],
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
            rewrite: "/mlflow",
          },
          {
            prefix: "/ai-gateway",
            cluster: "mlflow",
            rewrite: "/mlflow/gateway",
          },
          {
            prefix: "/v1/traces",
            cluster: "jaeger_otlp_http",
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
        name: "authelia",
        host: "authelia",
        port: 9091,
      },
      {
        name: "jaeger",
        host: "jaeger",
        port: 16686,
      },
      {
        name: "jaeger_otlp_http",
        host: "jaeger",
        port: 4318,
      },
      {
        name: "mlflow",
        host: "mlflow",
        port: 5000,
      },
      {
        name: "opa",
        host: "opa",
        port: 9191,
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
    auth: {
      cluster: "authelia",
      portal_prefix: "/authelia",
    },
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
    policy: {
      cluster: "opa",
    },
  },
  files: FIXTURE_FILES,
};
