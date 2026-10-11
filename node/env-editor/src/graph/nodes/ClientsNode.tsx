import { Handle, Position } from "@xyflow/react";
import { Monitor } from "lucide-react";
import { CLIENTS_SIZE } from "../useTopologyLayout";
import { HANDLE_CLASS } from "./BaseNode";

/**
 * The environment's clients (browsers, SDKs, CLIs on the host): where access
 * to the gateway starts. Not selectable and source-only. An actor rather than
 * a component, so a dashed pill, but with the same icon tile as the cards.
 * Fixed size — CLIENTS_SIZE in `useTopologyLayout`.
 */
export function ClientsNode() {
  return (
    <div
      className="flex items-center gap-2.5 rounded-full border border-dashed border-border bg-muted px-3"
      style={CLIENTS_SIZE}
    >
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-card text-muted-foreground">
        <Monitor className="h-4 w-4" />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold leading-5 text-card-foreground">
          Clients
        </div>
        <div className="truncate text-[11px] leading-4 text-muted-foreground">
          Browsers, SDKs, CLIs
        </div>
      </div>
      <Handle
        type="source"
        position={Position.Right}
        className={HANDLE_CLASS}
      />
    </div>
  );
}
