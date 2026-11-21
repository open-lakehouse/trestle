import { describe, expect, test } from "bun:test";
import type { HostedResource, Selection } from "../types.js";
import { migrateSelection } from "./migrate.js";

const selection = (s: Partial<Selection>): Selection => ({
  modules: [],
  capabilities: [],
  knob_overrides: {},
  ...s,
});

const fn = (url?: string): HostedResource => ({
  vendor: "neon-function",
  connection: { kind: "function", ...(url ? { url } : {}) },
});

describe("migrateSelection", () => {
  test("a current selection comes back as is", () => {
    const s = selection({ modules: ["unity-catalog-oss", "mlflow"] });
    expect(migrateSelection(s)).toBe(s);
  });

  test("a container Unity Catalog becomes the OSS server, with its settings", () => {
    const next = migrateSelection(
      selection({
        modules: ["mlflow", "unity-catalog"],
        knob_overrides: { "unity-catalog": { bucket: "lake" } },
      }),
    );
    expect(next.modules).toEqual(["mlflow", "unity-catalog-oss"]);
    expect(next.knob_overrides).toEqual({
      "unity-catalog-oss": { bucket: "lake" },
    });
  });

  test("a Unity Catalog function becomes mangrove's server", () => {
    const next = migrateSelection(
      selection({
        modules: ["unity-catalog"],
        knob_overrides: { "unity-catalog": { image: "oss:pinned" } },
        hosted: { "unity-catalog": fn() },
      }),
    );
    expect(next.modules).toEqual(["unity-catalog-rs"]);
    expect(next.hosted).toEqual({ "unity-catalog-rs": fn() });
    // The OSS server's image would be wrong for mangrove's.
    expect(next.knob_overrides).toEqual({});
  });

  test("with both ids, the new one is selected once and its settings win", () => {
    const next = migrateSelection(
      selection({
        modules: ["unity-catalog", "mlflow", "unity-catalog-oss"],
        knob_overrides: {
          "unity-catalog": { bucket: "lake", image: "old" },
          "unity-catalog-oss": { image: "new" },
        },
      }),
    );
    expect(next.modules).toEqual(["unity-catalog-oss", "mlflow"]);
    expect(next.knob_overrides).toEqual({
      "unity-catalog-oss": { bucket: "lake", image: "new" },
    });
  });

  test("an edited function entry under the new id wins", () => {
    const next = migrateSelection(
      selection({
        modules: ["unity-catalog-rs"],
        hosted: {
          "unity-catalog": fn("https://old.example.com"),
          "unity-catalog-rs": fn("https://new.example.com"),
        },
      }),
    );
    expect(next.hosted).toEqual({
      "unity-catalog-rs": fn("https://new.example.com"),
    });
  });
});

describe("migratePlatform (mirrors the Rust migrate_platform)", () => {
  test("moves legacy gateway settings to platform features", () => {
    const migrated = migrateSelection(
      selection({
        modules: ["mlflow"],
        gateway: { auth: false, policy: true },
      }),
    );
    expect(migrated.platform).toEqual({ identity: true, policy: true });
    expect(migrated.gateway).toBeUndefined();
  });

  test("turns listed providers and capabilities into their features", () => {
    const migrated = migrateSelection(
      selection({
        modules: ["mlflow", "jaeger", "opa"],
        capabilities: ["auth"],
      }),
    );
    expect(migrated.modules).toEqual(["mlflow"]);
    expect(migrated.capabilities).toEqual([]);
    expect(migrated.platform).toEqual({
      observability: "jaeger",
      policy: "opa",
      identity: true,
    });
  });

  test("an explicit sign-in off drops the listed provider", () => {
    const migrated = migrateSelection(
      selection({
        modules: ["envoy", "authelia"],
        knob_overrides: { envoy: { auth: "false", image: "envoy:pinned" } },
      }),
    );
    expect(migrated.modules).toEqual(["envoy"]);
    expect(migrated.platform).toEqual({});
    expect(migrated.knob_overrides).toEqual({
      envoy: { image: "envoy:pinned" },
    });
  });

  test("keeps a gateway that is off, and an already chosen provider", () => {
    const migrated = migrateSelection(
      selection({
        modules: ["jaeger"],
        gateway: { enabled: false, auth: false, policy: false },
        platform: { observability: "other-tracer" },
      }),
    );
    expect(migrated.gateway).toEqual({ enabled: false });
    expect(migrated.platform).toEqual({ observability: "other-tracer" });
  });

  test("leaves a current selection alone", () => {
    const current = selection({
      modules: ["mlflow"],
      platform: { identity: true },
    });
    expect(migrateSelection(current)).toBe(current);
  });
});
