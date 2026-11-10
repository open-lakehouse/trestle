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
