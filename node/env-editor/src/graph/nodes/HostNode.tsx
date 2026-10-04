import { Handle, Position } from "@xyflow/react";
import { Monitor } from "lucide-react";

/**
 * The host / browser: where ingress edges (host-published ports) start. Not
 * selectable and source-only. Fixed size (160×64) — MUST match HOST_SIZE in
 * `useTopologyLayout`.
 */
export function HostNode() {
  return (
    <div className="flex h-[64px] w-[160px] items-center justify-center gap-2 rounded-full border border-dashed border-border bg-muted text-sm font-medium text-muted-foreground">
      <Monitor className="h-4 w-4" />
      Host / browser
      <Handle
        type="source"
        position={Position.Right}
        className="!border-border !bg-muted-foreground"
      />
    </div>
  );
}
