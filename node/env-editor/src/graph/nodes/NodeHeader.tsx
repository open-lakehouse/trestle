import { cn } from "@open-lakehouse/ui-kit";
import { FileCode2 } from "lucide-react";
import type { ReactNode } from "react";
import { describeOrigin } from "../../state/origin";
import type { GraphNodeDto } from "../../types";
import { useCanvasContext } from "../canvasActions";
import { useNodeFiles } from "../nodeFiles";
import { useNodeSlots } from "../nodeSlots";
import { ModuleIcon } from "../projectIcons";
import { roleStyle } from "../roleTheme";
import { NODE_HEADER } from "../useTopologyLayout";

/**
 * The heading every card opens with: the project's logo on a tile tinted by
 * the role accent, the implementation's name, and the function it fills. On
 * an editable canvas, a component the planner pulled in carries an "Auto" chip;
 * a node with rendered files (when the host provides them) a count that opens
 * them.
 * A host's `NodeSlots` may render the name and role lines instead, and add
 * trailing content after the chips.
 * Fixed height (NODE_HEADER) — the layout reserves exactly this.
 */
export function NodeHeader({ node }: { node: GraphNodeDto }) {
  const style = roleStyle(node.role);
  const origin = useCanvasContext()?.actions.originOf(node.id);
  const slots = useNodeSlots();
  const files = useNodeFiles(node.id);
  const title = node.display_name ?? node.id;
  const name = slots?.name?.(node, title);
  const role = slots?.role?.(node, style.label);
  const aside = slots?.aside?.(node);
  const slotted =
    name !== undefined || role !== undefined || aside !== undefined;
  return (
    <div
      className="flex shrink-0 items-center gap-2.5 px-3"
      style={{ height: NODE_HEADER }}
      // A native tooltip would double the host's own popover.
      title={slotted ? undefined : (node.summary ?? title)}
    >
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-current/10",
          style.accent,
        )}
      >
        <ModuleIcon moduleId={node.id} role={node.role} className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold leading-5 text-card-foreground">
          {name === undefined ? title : <Slot>{name}</Slot>}
        </div>
        <div className="truncate text-[11px] leading-4 text-muted-foreground">
          {role === undefined ? style.label : <Slot>{role}</Slot>}
        </div>
      </div>
      {node.kind === "external" && <Chip>External</Chip>}
      {origin && origin.kind !== "explicit" && (
        <Chip title={describeOrigin(origin)}>Auto</Chip>
      )}
      {files && (
        <FilesChip count={files.paths.length} onOpen={() => files.open()} />
      )}
      {aside !== undefined && (
        <div className="shrink-0">
          <Slot>{aside}</Slot>
        </div>
      )}
    </div>
  );
}

/**
 * Host content inside a node: a link click must navigate, not select the node
 * or start a drag (`nodrag nopan` opt out of React Flow's pointer handling).
 */
function Slot({ children }: { children: ReactNode }) {
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: only stops propagation; the host's own element is the interactive one.
    // biome-ignore lint/a11y/useKeyWithClickEvents: see above.
    <span
      className="nodrag nopan"
      onClick={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
    >
      {children}
    </span>
  );
}

function Chip({ title, children }: { title?: string; children: string }) {
  return (
    <span
      className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-muted-foreground"
      title={title}
    >
      {children}
    </span>
  );
}

/**
 * How many config files a node renders, as a button that opens them. Like a
 * slot, it takes its own pointer events: a click neither selects nor drags.
 */
function FilesChip({ count, onOpen }: { count: number; onOpen: () => void }) {
  const label = `View ${count} config ${count === 1 ? "file" : "files"}`;
  return (
    <button
      type="button"
      className="nodrag nopan inline-flex h-5 shrink-0 items-center gap-1 rounded border border-border bg-background/60 px-1.5 text-[10px] font-semibold tabular-nums text-muted-foreground transition-colors hover:border-primary/40 hover:bg-primary/10 hover:text-primary focus-visible:outline-2 focus-visible:outline-ring"
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation();
        onOpen();
      }}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <FileCode2 className="h-3 w-3" />
      {count}
    </button>
  );
}
