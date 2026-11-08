import { describe, expect, test } from "bun:test";
import type { Selection } from "../types.js";
import { describeHostedChange, diffSelections, isEmptyDiff } from "./diff.js";

const base: Selection = {
  modules: ["postgres", "unity-catalog-oss"],
  capabilities: ["catalog"],
  knob_overrides: { postgres: { version: "16" } },
};

describe("diffSelections", () => {
  test("equal selections have an empty diff", () => {
    expect(isEmptyDiff(diffSelections(base, structuredClone(base)))).toBe(true);
  });

  test("modules and capabilities added and removed, sorted", () => {
    const diff = diffSelections(base, {
      ...base,
      modules: ["unity-catalog-oss", "rustfs", "mlflow"],
      capabilities: [],
    });
    expect(diff.modules).toEqual({
      added: ["mlflow", "rustfs"],
      removed: ["postgres"],
    });
    expect(diff.capabilities).toEqual({ added: [], removed: ["catalog"] });
    expect(isEmptyDiff(diff)).toBe(false);
  });

  test("knobs set, changed and cleared", () => {
    const diff = diffSelections(base, {
      ...base,
      knob_overrides: {
        postgres: { version: "17" },
        mlflow: { port: "5000" },
      },
    });
    expect(diff.knobs).toEqual([
      { module: "mlflow", knob: "port", from: undefined, to: "5000" },
      { module: "postgres", knob: "version", from: "16", to: "17" },
    ]);
    const cleared = diffSelections(base, { ...base, knob_overrides: {} });
    expect(cleared.knobs).toEqual([
      { module: "postgres", knob: "version", from: "16", to: undefined },
    ]);
  });

  test("whole settings compare by value, ignoring key order", () => {
    const a: Selection = {
      ...base,
      gateway: { enabled: true, auth: "none" } as Selection["gateway"],
    };
    const b: Selection = {
      ...base,
      gateway: { auth: "none", enabled: true } as Selection["gateway"],
    };
    expect(diffSelections(a, b).settings).toEqual([]);
    expect(
      diffSelections(a, { ...b, client: { python: true } }).settings,
    ).toEqual(["client"]);
    expect(diffSelections(a, base).settings).toEqual(["gateway"]);
  });
});

describe("hosted changes", () => {
  const fn: Selection = {
    ...base,
    hosted: {
      "unity-catalog-rs": {
        vendor: "neon-function",
        connection: { kind: "function" },
      },
    },
  };
  const rows = (a: Selection, b: Selection) =>
    diffSelections(a, b).hosted.map(describeHostedChange);

  test("running a module as a Neon Function reads as such", () => {
    const diff = diffSelections(base, fn);
    expect(diff.settings).toEqual([]);
    expect(diff.hosted).toEqual([
      {
        module: "unity-catalog-rs",
        from: undefined,
        to: { vendor: "neon-function", connection: { kind: "function" } },
      },
    ]);
    expect(isEmptyDiff(diff)).toBe(false);
    expect(rows(base, fn)).toEqual([
      "unity-catalog-rs runs as a Neon Function",
    ]);
  });

  test("switching back runs it in a local container", () => {
    expect(rows(fn, base)).toEqual([
      "unity-catalog-rs runs in a local container",
    ]);
  });

  test("a function URL change shows both URLs", () => {
    const withUrl: Selection = {
      ...base,
      hosted: {
        "unity-catalog-rs": {
          vendor: "neon-function",
          connection: { kind: "function", url: "https://uc.example" },
        },
      },
    };
    expect(rows(fn, withUrl)).toEqual([
      "unity-catalog-rs function URL: (default) → https://uc.example",
    ]);
  });

  test("a hosted provider's vendor change names both vendors", () => {
    const pg = (vendor?: "neon" | "aws-rds"): Selection => ({
      ...base,
      hosted: {
        "hosted-postgres": {
          vendor,
          connection: {
            kind: "postgres",
            host: "db",
            username: "u",
            password: "pw",
          },
        },
      },
    });
    expect(rows(base, pg("neon"))).toEqual([
      "hosted-postgres connection: (default) → Neon",
    ]);
    expect(rows(pg("neon"), pg("aws-rds"))).toEqual([
      "hosted-postgres connection: Neon → Amazon RDS",
    ]);
    expect(rows(pg("neon"), structuredClone(pg("neon")))).toEqual([]);
  });
});
