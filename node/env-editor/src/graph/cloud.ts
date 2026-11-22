import type { GraphNodeDto } from "../types.js";
import type { CloudGroupNodeData } from "./nodes/CloudGroupNode.js";

/** The name the group of `cloud` is shown under: the planner's `cloud_name`
 *  ("Neon", "Databricks", "AWS", "Azure", "Hosted"). A snapshot saved before
 *  the planner named clouds has only the raw label; it shows as is, and an
 *  unnamed cloud ("") as "Hosted". */
export function cloudName(cloud: string, name?: string | null): string {
  return name ?? (cloud || "Hosted");
}

/** Whether `node` is one of the environment's modules: not the clients, nor a
 *  service outside the environment that a module calls (a cloud's AI
 *  gateway). Both of those have synthetic ids, which start with `@`. */
export function isModuleNode(node: GraphNodeDto): boolean {
  return !node.id.startsWith("@");
}

/** The frames of a graph's clouds, in order of their first member: each
 *  cloud's label, the name its members carry, and its members (hosted
 *  providers and the services outside the environment). */
export function cloudFrames(
  nodes: GraphNodeDto[],
): Map<string, CloudGroupNodeData & { members: GraphNodeDto[] }> {
  const frames = new Map<
    string,
    CloudGroupNodeData & { members: GraphNodeDto[] }
  >();
  for (const n of nodes) {
    if (n.cloud == null) continue;
    const frame = frames.get(n.cloud) ?? { cloud: n.cloud, members: [] };
    frame.name ??= n.cloud_name;
    frame.members.push(n);
    frames.set(n.cloud, frame);
  }
  return frames;
}
