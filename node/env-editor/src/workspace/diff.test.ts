import { describe, expect, test } from "bun:test";
import type { Selection } from "../types.js";
import { diffSelections, isEmptyDiff } from "./diff.js";

const base: Selection = {
  modules: ["postgres", "unity-catalog"],
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
      modules: ["unity-catalog", "rustfs", "mlflow"],
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
