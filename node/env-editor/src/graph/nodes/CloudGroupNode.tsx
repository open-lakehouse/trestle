import type { NodeProps } from "@xyflow/react";
import { Cloud } from "lucide-react";
import { CLOUD_GROUP_HEADER } from "../useTopologyLayout";

/** What a cloud group node carries: the cloud's label ("" when unnamed). */
export interface CloudGroupNodeData {
  cloud: string;
  [key: string]: unknown;
}

/** The title of the group for `cloud` ("" when the external environment is unnamed). */
export function cloudGroupTitle(cloud: string): string {
  return cloud ? `External cloud · ${cloud.toUpperCase()}` : "External cloud";
}

/**
 * The outer frame around the resources an external cloud provides (a managed
 * database, a cloud object store): the stack uses them but does not run them.
 * A dashed, inert container — its member cards are child nodes, laid out
 * inside it by ELK, and the frame's size is ELK's.
 */
export function CloudGroupNode({ data, width, height }: NodeProps) {
  const { cloud } = data as CloudGroupNodeData;
  return (
    <div
      className="pointer-events-none rounded-xl border-2 border-dashed border-sky-500/40 bg-sky-500/5"
      style={{ width, height }}
    >
      <div
        className="flex items-center gap-1.5 px-3 text-[11px] font-semibold uppercase tracking-wide text-sky-700 dark:text-sky-300"
        style={{ height: CLOUD_GROUP_HEADER }}
      >
        <Cloud className="h-3.5 w-3.5" />
        {cloudGroupTitle(cloud)}
      </div>
    </div>
  );
}
