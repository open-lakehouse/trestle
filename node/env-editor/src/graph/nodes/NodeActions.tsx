import {
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { NodeToolbar, Position } from "@xyflow/react";
import {
  ArrowLeftRight,
  FileCode2,
  Lock,
  Settings2,
  Trash2,
} from "lucide-react";
import {
  type ComponentType,
  type MouseEvent,
  useEffect,
  useState,
} from "react";
import { describeOrigin } from "../../state/origin";
import { useCanvasContext } from "../canvasActions";
import { useNodeFiles } from "../nodeFiles";
import { ModuleIcon } from "../projectIcons";

/**
 * The action bar floating above a node on hover or selection: configure, view
 * its config files, swap for another module filling the same role, and remove. A component the
 * planner pulled in shows a lock instead of remove, explaining what needs it.
 * Renders nothing when the canvas is read-only.
 */
export function NodeActions({
  id,
  selected,
}: {
  id: string;
  selected?: boolean;
}) {
  const ctx = useCanvasContext();
  const files = useNodeFiles(id);
  const [swapOpen, setSwapOpen] = useState(false);
  const visible = !!ctx && (selected || ctx.hoveredId === id);

  useEffect(() => {
    if (!visible) setSwapOpen(false);
  }, [visible]);

  if (!ctx) return null;
  const { actions, hover, leave } = ctx;
  const origin = actions.originOf(id);
  const alternatives = actions.alternativesOf(id);
  const hasSettings = actions.hasSettings(id);

  // The toolbar sits in a portal but still bubbles React events to the node:
  // keep its clicks from (re)selecting the node.
  const run = (fn: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    fn();
  };

  return (
    <NodeToolbar isVisible={visible} position={Position.Top} offset={8}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: hover keeps the bar open while the pointer travels onto it */}
      <div
        className="nodrag nopan flex flex-col items-center gap-1"
        onMouseEnter={() => hover(id)}
        onMouseLeave={leave}
      >
        {swapOpen && (
          <div className="flex min-w-48 flex-col rounded-md border border-border bg-card p-1 text-card-foreground shadow-md">
            <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              Replace with
            </div>
            {alternatives.map((m) => (
              <button
                key={m.id}
                type="button"
                className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-muted"
                onClick={run(() => {
                  setSwapOpen(false);
                  actions.swap(id, m.id);
                })}
              >
                <ModuleIcon
                  moduleId={m.id}
                  role={m.provider_of}
                  className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                />
                <span className="truncate">{m.display_name ?? m.id}</span>
              </button>
            ))}
          </div>
        )}
        <div className="flex items-center gap-0.5 rounded-md border border-border bg-card p-0.5 text-card-foreground shadow-md">
          <ActionButton
            icon={Settings2}
            label={hasSettings ? "Configure" : "No settings to configure"}
            disabled={!hasSettings}
            onClick={run(() => actions.configure(id))}
          />
          {files && (
            <ActionButton
              icon={FileCode2}
              label="View config files"
              onClick={run(() => files.open())}
            />
          )}
          <ActionButton
            icon={ArrowLeftRight}
            label={
              alternatives.length > 0
                ? "Replace with another module"
                : "No alternatives in the catalog"
            }
            disabled={alternatives.length === 0}
            active={swapOpen}
            onClick={run(() => setSwapOpen((open) => !open))}
          />
          {origin.kind === "explicit" ? (
            <ActionButton
              icon={Trash2}
              label="Remove"
              destructive
              onClick={run(() => actions.remove(id))}
            />
          ) : (
            <ActionButton
              icon={Lock}
              label={`${describeOrigin(origin)}, so it can't be removed on its own`}
              disabled
            />
          )}
        </div>
      </div>
    </NodeToolbar>
  );
}

function ActionButton({
  icon: Icon,
  label,
  disabled,
  destructive,
  active,
  onClick,
}: {
  icon: ComponentType<{ className?: string }>;
  label: string;
  disabled?: boolean;
  destructive?: boolean;
  active?: boolean;
  onClick?: (e: MouseEvent) => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* A span keeps the tooltip working on a disabled button. */}
        <span>
          <button
            type="button"
            aria-label={label}
            disabled={disabled}
            onClick={onClick}
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40",
              destructive && "hover:bg-destructive/10 hover:text-destructive",
              active && "bg-muted text-foreground",
            )}
          >
            <Icon className="h-3.5 w-3.5" />
          </button>
        </span>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-64 text-xs">
        {label}
      </TooltipContent>
    </Tooltip>
  );
}
