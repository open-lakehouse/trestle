// A workspace: several environments sharing one Docker host, each on its own
// slot. This mirrors the Rust `WorkspaceManifest` (crates/stack-topology/src/
// workspace.rs): the same names, slots, port shifts and project names, so a
// workspace made here renders with `trestle workspace render` unchanged.

import type { HostPortDto, PlanContext, Selection } from "../types";

/** One environment of a workspace, as the editor keeps it. */
export interface EnvDoc {
  /** Stable id (survives renames); the editor's tab key. */
  id: string;
  /** The environment's name: its directory, hostnames and project suffix. */
  name: string;
  /** Its slot: host ports sit `slot × SLOT_PORT_STRIDE` above the defaults. */
  slot: number;
  /** The modules and settings chosen for it; the catalog's default until the
   *  first edit. */
  selection?: Selection;
}

/** A workspace as the editor keeps it (see {@link WorkspaceStore}). */
export interface WorkspaceDoc {
  /** The workspace's name, prefixing every member's compose project. */
  name: string;
  /** Whether the members' gateways share a network. */
  mesh: boolean;
  envs: EnvDoc[];
}

/** How far apart two adjacent slots' host ports are (`SLOT_PORT_STRIDE`). */
export const SLOT_PORT_STRIDE = 1000;
/** The highest slot (`MAX_SLOT`), keeping shifted ports below 65535. */
export const MAX_SLOT = 54;

/** The single-environment defaults the slots shift (`PlanCtx::default()`). */
const DEFAULT_PORTS = {
  gateway: 9080,
  admin: 9901,
  dedicatedBase: 9100,
} as const;

/** `^[a-z][a-z0-9-]*$`, at most 63 characters (`is_valid_name`). */
export function isValidName(name: string): boolean {
  return /^[a-z][a-z0-9-]*$/.test(name) && name.length <= 63;
}

/** The lowest slot no member holds, or `undefined` when all are taken. */
export function nextSlot(workspace: WorkspaceDoc): number | undefined {
  for (let slot = 0; slot <= MAX_SLOT; slot++) {
    if (!workspace.envs.some((e) => e.slot === slot)) return slot;
  }
  return undefined;
}

/** Why `name` can't name a new (or renamed) member of `workspace`, if it can't. */
export function nameProblem(
  workspace: WorkspaceDoc,
  name: string,
  except?: string,
): string | undefined {
  if (!isValidName(name)) {
    return "Use lowercase letters, digits and dashes, starting with a letter.";
  }
  if (workspace.envs.some((e) => e.name === name && e.id !== except)) {
    return `The workspace already has an environment \`${name}\`.`;
  }
  return undefined;
}

/** A free member name derived from `base` (`base`, `base-2`, …). */
export function freeName(workspace: WorkspaceDoc, base: string): string {
  const taken = new Set(workspace.envs.map((e) => e.name));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}-${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** The compose project of `env`: `<workspace>-<env>`. */
export function projectName(workspace: WorkspaceDoc, env: EnvDoc): string {
  return `${workspace.name}-${env.name}`;
}

/** The shared network, when the mesh is on: `trestle-<workspace>-mesh`. */
export function meshNetwork(workspace: WorkspaceDoc): string | undefined {
  return workspace.mesh ? `trestle-${workspace.name}-mesh` : undefined;
}

/** The context `env` plans under (`WorkspaceManifest::apply`). */
export function contextFor(workspace: WorkspaceDoc, env: EnvDoc): PlanContext {
  const offset = env.slot * SLOT_PORT_STRIDE;
  return {
    env_name: env.name,
    project_name: projectName(workspace, env),
    mesh_network: meshNetwork(workspace),
    port_offset: offset,
    gateway_host_port: DEFAULT_PORTS.gateway + offset,
    gateway_admin_port: DEFAULT_PORTS.admin + offset,
    dedicated_listener_port_base: DEFAULT_PORTS.dedicatedBase + offset,
  };
}

/** A host port two members both bind (`PortConflict`). */
export interface PortConflict {
  port: number;
  first: { env: string; owner: string };
  second: { env: string; owner: string };
}

/** Every host port bound by more than one member, sorted by port. */
export function portConflicts(
  members: readonly { env: string; hostPorts: readonly HostPortDto[] }[],
): PortConflict[] {
  const owners = new Map<number, { env: string; owner: string }>();
  const conflicts: PortConflict[] = [];
  for (const { env, hostPorts } of members) {
    for (const { port, owner } of hostPorts) {
      const first = owners.get(port);
      if (!first) owners.set(port, { env, owner });
      else if (first.env !== env) {
        conflicts.push({ port, first, second: { env, owner } });
      }
    }
  }
  return conflicts.sort((a, b) => a.port - b.port);
}

/** `value` as a TOML basic string. */
function tomlString(value: string): string {
  return JSON.stringify(value);
}

/** The workspace's `trestle-workspace.toml`, as the CLI writes it. */
export function workspaceToml(workspace: WorkspaceDoc): string {
  const lines = [
    "version = 1",
    `name = ${tomlString(workspace.name)}`,
    `mesh = ${workspace.mesh}`,
  ];
  for (const env of workspace.envs) {
    lines.push(
      "",
      "[[envs]]",
      `name = ${tomlString(env.name)}`,
      `path = ${tomlString(env.name)}`,
      `slot = ${env.slot}`,
    );
  }
  return `${lines.join("\n")}\n`;
}
