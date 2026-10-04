import { Handle, Position } from "@xyflow/react";
import { Monitor } from "lucide-react";

/**
 * The environment's clients (browsers, SDKs, CLIs on the host): where access
 * to the gateway starts. Not selectable and source-only. Fixed size (160×64) —
 * MUST match CLIENTS_SIZE in `useTopologyLayout`.
 */
export function ClientsNode() {
  return (
    <div className="flex h-[64px] w-[160px] items-center justify-center gap-2 rounded-full border border-dashed border-border bg-muted text-sm font-medium text-muted-foreground">
      <Monitor className="h-4 w-4" />
      Clients
      <Handle
        type="source"
        position={Position.Right}
        className="!border-border !bg-muted-foreground"
      />
    </div>
  );
}
