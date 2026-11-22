import type { NodeProps } from "@xyflow/react";
import { Cloud } from "lucide-react";
import { cloudName } from "../cloud.js";
import { CLOUD_GROUP_HEADER } from "../useTopologyLayout.js";

/** What a cloud group node carries: the cloud's label ("" when unnamed), and
 *  the name the planner gives it (absent in older snapshots). */
export interface CloudGroupNodeData {
  cloud: string;
  name?: string;
  [key: string]: unknown;
}

/** The title of the group for `cloud`, named `name` by the planner. */
export function cloudGroupTitle(cloud: string, name?: string): string {
  return cloudName(cloud, name);
}

/**
 * The outer frame around what a cloud outside the environment provides (a
 * managed database, an object store, an AI gateway): the stack uses them but
 * does not run them.
 * A dashed, inert container — its member cards are child nodes, laid out
 * inside it by ELK, and the frame's size is ELK's.
 */
export function CloudGroupNode({ data, width, height }: NodeProps) {
  const { cloud, name } = data as CloudGroupNodeData;
  return (
    <div
      className="pointer-events-none rounded-xl border-2 border-dashed border-sky-500/40 bg-sky-500/5"
      style={{ width, height }}
    >
      <div
        className="flex items-center gap-1.5 px-3 text-[11px] font-semibold tracking-wide text-sky-700 dark:text-sky-300"
        style={{ height: CLOUD_GROUP_HEADER }}
      >
        <Cloud className="h-3.5 w-3.5" />
        {cloudGroupTitle(cloud, name)}
      </div>
    </div>
  );
}
