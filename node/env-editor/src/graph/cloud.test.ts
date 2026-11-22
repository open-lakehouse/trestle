import { describe, expect, test } from "bun:test";
import type { GraphNodeDto, TopologyEdgeDto } from "../types.js";
import { cloudFrames, cloudName, isModuleNode } from "./cloud.js";
import { edgeLabel, edgeTitle } from "./MarkitectureCanvas.js";
import { cloudGroupTitle } from "./nodes/CloudGroupNode.js";

const node = (
  id: string,
  kind: GraphNodeDto["kind"],
  cloud?: string,
  role = "experiment_tracking",
): GraphNodeDto => ({
  id,
  kind,
  display_name: id,
  role,
  cloud: cloud ?? null,
  // As the planner names them.
  cloud_name:
    cloud === undefined
      ? undefined
      : { neon: "Neon", databricks: "Databricks" }[cloud],
  offers: [],
  surfaces: [],
  provisions: [],
  resource_nouns: [],
});

const uses = (
  from: string,
  to: string,
  auth?: "native" | "token",
): TopologyEdgeDto => ({
  key: `uses:${from}→${to}:ai_gateway`,
  kind: "uses",
  from,
  to,
  role: "ai_gateway",
  protocol: "HTTP",
  resources: [],
  auth,
});

describe("cloud names", () => {
  test("the planner's name wins", () => {
    expect(cloudName("neon", "Neon")).toBe("Neon");
    expect(cloudGroupTitle("databricks", "Databricks")).toBe("Databricks");
  });

  test("an older snapshot shows the raw label, or Hosted when unnamed", () => {
    expect(cloudName("neon")).toBe("neon");
    expect(cloudGroupTitle("")).toBe("Hosted");
  });

  test("only modules are module nodes", () => {
    expect(isModuleNode(node("mlflow", "component"))).toBe(true);
    expect(isModuleNode(node("@clients", "clients"))).toBe(false);
    expect(isModuleNode(node("@neon-ai-gateway", "external", "neon"))).toBe(
      false,
    );
  });
});

describe("outside edges", () => {
  test("a uses edge leaving the environment is labelled with its auth", () => {
    const e = uses("mlflow", "@neon-ai-gateway", "token");
    expect(edgeLabel(e)).toBe("token");
    expect(edgeTitle(e, new Set())).toBe(
      "mlflow uses @neon-ai-gateway (HTTP, token auth) as its ai gateway",
    );
  });

  test("an internal uses edge stays unlabelled", () => {
    expect(edgeLabel(uses("mlflow", "postgres"))).toBeUndefined();
  });
});

describe("cloud frames", () => {
  test("outside services share their cloud's frame and name", () => {
    const frames = cloudFrames([
      node("mlflow", "component"),
      node("@neon-ai-gateway", "external", "neon", "ai_gateway"),
      node("@databricks-ai-gateway", "external", "databricks", "ai_gateway"),
      node(
        "@databricks-sandbox-host",
        "external",
        "databricks",
        "sandbox_host",
      ),
    ]);
    expect([...frames.keys()]).toEqual(["neon", "databricks"]);
    expect(frames.get("neon")?.name).toBe("Neon");
    const databricks = frames.get("databricks");
    expect(databricks?.name).toBe("Databricks");
    expect(databricks?.members.map((n) => n.id)).toEqual([
      "@databricks-ai-gateway",
      "@databricks-sandbox-host",
    ]);
  });
});
