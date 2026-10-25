import { describe, expect, test } from "bun:test";
import {
  contextFor,
  type EnvDoc,
  freeName,
  isValidName,
  MAX_SLOT,
  nameProblem,
  nextSlot,
  portConflicts,
  type WorkspaceDoc,
  workspaceToml,
} from "./model";

const env = (name: string, slot: number): EnvDoc => ({ id: name, name, slot });
const demo: WorkspaceDoc = {
  name: "demo",
  mesh: true,
  envs: [env("dev", 0), env("prod", 1)],
};

describe("workspace model", () => {
  test("names follow the CLI's rule", () => {
    expect(isValidName("prod-2")).toBe(true);
    expect(isValidName("Prod")).toBe(false);
    expect(isValidName("2prod")).toBe(false);
    expect(nameProblem(demo, "dev")).toContain("already");
    expect(nameProblem(demo, "dev", "dev")).toBeUndefined();
    expect(freeName(demo, "dev")).toBe("dev-2");
  });

  test("slots are handed out lowest free first", () => {
    expect(nextSlot(demo)).toBe(2);
    expect(nextSlot({ ...demo, envs: [env("prod", 1)] })).toBe(0);
    const full = Array.from({ length: MAX_SLOT + 1 }, (_, i) =>
      env(`e${i}`, i),
    );
    expect(nextSlot({ ...demo, envs: full })).toBeUndefined();
  });

  test("a member's context matches WorkspaceManifest::apply", () => {
    expect(contextFor(demo, env("prod", 1))).toEqual({
      env_name: "prod",
      project_name: "demo-prod",
      mesh_network: "trestle-demo-mesh",
      port_offset: 1000,
      gateway_host_port: 10080,
      gateway_admin_port: 10901,
      dedicated_listener_port_base: 10100,
    });
    // Slot 0 keeps the single-environment ports; no mesh leaves it unset.
    const dev = contextFor({ ...demo, mesh: false }, env("dev", 0));
    expect(dev.gateway_host_port).toBe(9080);
    expect(dev.mesh_network).toBeUndefined();
  });

  test("conflicts are ports two members share", () => {
    const conflicts = portConflicts([
      { env: "dev", hostPorts: [{ port: 9080, owner: "envoy" }] },
      {
        env: "prod",
        hostPorts: [
          { port: 9080, owner: "envoy" },
          { port: 10080, owner: "envoy" },
        ],
      },
    ]);
    expect(conflicts).toEqual([
      {
        port: 9080,
        first: { env: "dev", owner: "envoy" },
        second: { env: "prod", owner: "envoy" },
      },
    ]);
  });

  test("the manifest is what `trestle workspace` reads", () => {
    expect(workspaceToml(demo)).toBe(
      [
        "version = 1",
        'name = "demo"',
        "mesh = true",
        "",
        "[[envs]]",
        'name = "dev"',
        'path = "dev"',
        "slot = 0",
        "",
        "[[envs]]",
        'name = "prod"',
        'path = "prod"',
        "slot = 1",
        "",
      ].join("\n"),
    );
  });
});
