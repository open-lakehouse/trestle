import { cn } from "@open-lakehouse/ui-kit";
import { describeOrigin } from "../../state/origin";
import type { GraphNodeDto } from "../../types";
import { useCanvasContext } from "../canvasActions";
import { ModuleIcon } from "../projectIcons";
import { roleStyle } from "../roleTheme";
import { NODE_HEADER } from "../useTopologyLayout";

/**
 * The heading every card opens with: the project's logo on a tile tinted by
 * the role accent, the implementation's name, and the function it fills. On
 * an editable canvas, a component the planner pulled in carries an "Auto" chip.
 * Fixed height (NODE_HEADER) — the layout reserves exactly this.
 */
export function NodeHeader({ node }: { node: GraphNodeDto }) {
  const style = roleStyle(node.role);
  const origin = useCanvasContext()?.actions.originOf(node.id);
  const title = node.display_name ?? node.id;
  return (
    <div
      className="flex shrink-0 items-center gap-2.5 px-3"
      style={{ height: NODE_HEADER }}
      title={node.summary ?? title}
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
          {title}
        </div>
        <div className="truncate text-[11px] leading-4 text-muted-foreground">
          {style.label}
        </div>
      </div>
      {node.kind === "external" && <Chip>External</Chip>}
      {origin && origin.kind !== "explicit" && (
        <Chip title={describeOrigin(origin)}>Auto</Chip>
      )}
    </div>
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
