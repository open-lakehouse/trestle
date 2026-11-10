import { describe, expect, test } from "bun:test";
import type { Selection } from "../types.js";
import {
  compareSelections,
  describeHostedChange,
  diffFiles,
  diffSelections,
  isEmptyDiff,
  isSameEnvironment,
} from "./diff.js";

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

describe("diffFiles", () => {
  const f = (path: string, contents: string) => ({ path, contents });
  test("added, removed and changed files, by path; equal ones left out", () => {
    expect(
      diffFiles(
        [f("b.yaml", "1"), f("same", "x"), f("gone", "old")],
        [f("same", "x"), f("b.yaml", "2"), f("a.new", "hi")],
      ),
    ).toEqual([
      { path: "a.new", original: undefined, modified: "hi" },
      { path: "b.yaml", original: "1", modified: "2" },
      { path: "gone", original: "old", modified: undefined },
    ]);
  });

  test("equal plans have no changes", () => {
    expect(diffFiles([f("a", "1")], [f("a", "1")])).toEqual([]);
  });
});

describe("compareSelections", () => {
  const file = (path: string, contents: string) => ({ path, contents });
  /** A planner whose files follow the selection: one per module. */
  const plan = async (s: Selection) => ({
    files: s.modules.map((m) =>
      file(
        `modules/${m}/compose.yaml`,
        JSON.stringify(s.knob_overrides[m] ?? {}),
      ),
    ),
  });
  const failing = (bad: Selection) => async (s: Selection) => {
    if (s === bad) throw new Error("unknown module `gone`");
    return plan(s);
  };

  test("lists the selection change and the files, before to after", async () => {
    const after = { ...base, modules: [...base.modules, "mlflow"] };
    const c = await compareSelections(base, after, plan);
    expect(c.selection.modules.added).toEqual(["mlflow"]);
    expect(c.files).toEqual([
      {
        path: "modules/mlflow/compose.yaml",
        original: undefined,
        modified: "{}",
      },
    ]);
    expect(isSameEnvironment(c)).toBe(false);
  });

  test("a change only in the files is not the same environment", async () => {
    let n = 0;
    const drifting = async () => ({ files: [file("README.md", `${n++}`)] });
    const c = await compareSelections(base, base, drifting);
    expect(isEmptyDiff(c.selection)).toBe(true);
    expect(c.files).toEqual([
      { path: "README.md", original: "0", modified: "1" },
    ]);
    expect(isSameEnvironment(c)).toBe(false);
  });

  test("identical sides are the same environment", async () => {
    expect(isSameEnvironment(await compareSelections(base, base, plan))).toBe(
      true,
    );
  });

  test("a side that no longer plans keeps the selection diff", async () => {
    const old = { ...base, modules: ["gone"] };
    for (const [before, after] of [
      [old, base],
      [base, old],
    ] as const) {
      const c = await compareSelections(before, after, failing(old));
      expect(c.files).toEqual({ error: "unknown module `gone`" });
      expect(isEmptyDiff(c.selection)).toBe(false);
      expect(isSameEnvironment(c)).toBe(false);
    }
  });
});
