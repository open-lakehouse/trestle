import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { EnvIndicator, environmentIndicator } from "./envIndicator.js";
import type { MemberPlan } from "./useMemberPlans.js";

const plans = { plan: {} } as unknown as MemberPlan;
const broken = { error: "no such module" } as unknown as MemberPlan;

describe("environmentIndicator", () => {
  test("a plan problem first, then the host's state, then the plan", () => {
    const kind = (args: Parameters<typeof environmentIndicator>[0]) =>
      environmentIndicator(args).kind;
    expect(kind({ plan: undefined })).toBe("planning");
    expect(kind({ plan: plans })).toBe("ready");
    expect(kind({ plan: broken, status: { state: "running" } })).toBe(
      "plan-error",
    );
    expect(kind({ plan: plans, clashing: true })).toBe("clash");
    expect(kind({ plan: plans, status: { state: "running" } })).toBe("running");
    expect(kind({ plan: plans, status: { state: "starting" } })).toBe(
      "starting",
    );
    expect(kind({ plan: plans, status: { state: "failed" } })).toBe("failed");
    // Stopped is ready, when it plans.
    expect(kind({ plan: plans, status: { state: "stopped" } })).toBe("ready");
  });

  test("a draft no revision holds is dirty, and says so", () => {
    const changed = environmentIndicator({
      plan: plans,
      status: { state: "running" },
      draft: "changed",
    });
    expect(changed.kind).toBe("running");
    expect(changed.dirty).toBe(true);
    expect(changed.title).toContain("uncommitted changes");
    expect(
      environmentIndicator({ plan: plans, draft: "uncommitted" }).title,
    ).toContain("not committed yet");
    expect(
      environmentIndicator({ plan: plans, draft: "committed" }).dirty,
    ).toBe(false);
  });
});

describe("EnvIndicator", () => {
  test("a ring circles while it runs; the dot marks a dirty draft", () => {
    const out = renderToStaticMarkup(
      <EnvIndicator
        indicator={{ kind: "running", dirty: true, title: "running" }}
        runState="running"
      />,
    );
    expect(out).toContain('data-run-state="running"');
    expect(out).toContain('data-dirty="true"');
    expect(out).toContain("animate-spin");
    expect(out).toContain('aria-label="running"');
  });

  test("ready is a still check, with no run state of its own", () => {
    const out = renderToStaticMarkup(
      <EnvIndicator
        indicator={{ kind: "ready", dirty: false, title: "plans" }}
      />,
    );
    expect(out).toContain('data-indicator="ready"');
    expect(out).not.toContain("animate-spin");
    expect(out).not.toContain("data-run-state");
    expect(out).not.toContain("data-dirty");
  });
});
