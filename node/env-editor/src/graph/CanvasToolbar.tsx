import {
  Button,
  cn,
  Label,
  Switch,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Panel } from "@xyflow/react";
import {
  AlertTriangle,
  FileArchive,
  FileImage,
  FileJson,
  ImageDown,
  Info,
  Loader2,
  Plus,
  Redo2,
  Undo2,
  X,
} from "lucide-react";
import type { ReactNode } from "react";
import {
  type DiagramImageFormat,
  downloadText,
  downloadUrl,
  useDiagramImage,
} from "../diagram/exportImage";
import {
  createGraphSnapshot,
  GRAPH_SNAPSHOT_EXTENSION,
  stringifyGraphSnapshot,
} from "../diagram/snapshot";
import type { DeployStepDto, GraphDto } from "../types";

export interface CanvasToolbarProps {
  onAdd: () => void;
  onUndo: () => void;
  onRedo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  /** A replan is pending or running. */
  planning: boolean;
  /** Components in the current plan. */
  componentCount: number;
  /** The latest plan failed. */
  hasError: boolean;
  /** The planned graph, for the export actions (disabled while absent). */
  graph?: GraphDto;
  /**
   * The plan's viewable file paths, saved in the snapshot as its manifest so
   * an embed can show each node's files from the companion render.
   */
  files?: readonly string[];
  /** The plan's deployment steps, saved in the snapshot for downstream tools. */
  deploy?: readonly DeployStepDto[];
  /** The exported files' base name. Defaults to `"environment"`. */
  exportName?: string;
  /** Called with a message when an export fails. */
  onExportError?: (message: string) => void;
  /** Download the whole rendered environment. The action shows only with
   *  this. */
  onDownloadBundle?: () => void;
  /** A download is being prepared. */
  bundling?: boolean;
  /** The download action's tooltip. */
  bundleLabel?: string;
  /** Whether the environment runs a gateway. The toggle shows only with
   *  `onToggleGateway`. */
  gatewayEnabled?: boolean;
  /** Turn the gateway on or off. Off publishes each service on its own host
   *  port; this is the only way back when no gateway node is drawn. */
  onToggleGateway?: (enabled: boolean) => void;
}

const MOD_KEY =
  typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform)
    ? "⌘"
    : "Ctrl+";

/**
 * The canvas's top-left bar: add a component, undo/redo, export the diagram,
 * download the environment, and the planner's status. Render it as a child of `MarkitectureCanvas`.
 */
export function CanvasToolbar({
  onAdd,
  onUndo,
  onRedo,
  canUndo,
  canRedo,
  planning,
  componentCount,
  hasError,
  graph,
  files,
  deploy,
  exportName = "environment",
  onExportError,
  onDownloadBundle,
  bundling = false,
  bundleLabel = "Download environment (.zip)",
  gatewayEnabled = true,
  onToggleGateway,
}: CanvasToolbarProps) {
  return (
    <Panel position="top-left">
      <div className="flex items-center gap-1 rounded-lg border border-border bg-card/95 p-1 shadow-sm">
        <Button type="button" size="sm" className="h-8 gap-1.5" onClick={onAdd}>
          <Plus className="h-4 w-4" />
          Add component
          <kbd className="ml-1 rounded bg-primary-foreground/20 px-1 text-[10px] font-medium">
            {MOD_KEY}K
          </kbd>
        </Button>
        <div className="mx-1 h-5 w-px bg-border" />
        <IconAction
          label={`Undo (${MOD_KEY}Z)`}
          disabled={!canUndo}
          onClick={onUndo}
        >
          <Undo2 className="h-4 w-4" />
        </IconAction>
        <IconAction
          label={`Redo (${MOD_KEY}⇧Z)`}
          disabled={!canRedo}
          onClick={onRedo}
        >
          <Redo2 className="h-4 w-4" />
        </IconAction>
        <div className="mx-1 h-5 w-px bg-border" />
        <ExportActions
          graph={graph}
          files={files}
          deploy={deploy}
          name={exportName}
          onError={onExportError}
        />
        {onDownloadBundle && (
          <IconAction
            label={bundleLabel}
            disabled={hasError || planning || bundling || componentCount === 0}
            onClick={onDownloadBundle}
          >
            {bundling ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <FileArchive className="h-4 w-4" />
            )}
          </IconAction>
        )}
        <div className="mx-1 h-5 w-px bg-border" />
        {onToggleGateway && (
          <>
            <GatewayToggle
              enabled={gatewayEnabled}
              onToggle={onToggleGateway}
            />
            <div className="mx-1 h-5 w-px bg-border" />
          </>
        )}
        <span
          className={cn(
            "inline-flex items-center gap-1.5 px-2 text-xs",
            hasError && !planning
              ? "text-destructive"
              : "text-muted-foreground",
          )}
          aria-live="polite"
        >
          {planning ? (
            <>
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Planning…
            </>
          ) : hasError ? (
            <>
              <AlertTriangle className="h-3.5 w-3.5" />
              Plan failed
            </>
          ) : (
            `${componentCount} component${componentCount === 1 ? "" : "s"}`
          )}
        </span>
      </div>
    </Panel>
  );
}

/** Run the environment with or without a gateway. */
function GatewayToggle({
  enabled,
  onToggle,
}: {
  enabled: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className="flex items-center gap-1.5 px-1.5">
          <Switch
            id="canvas.gateway"
            checked={enabled}
            onCheckedChange={onToggle}
          />
          <Label htmlFor="canvas.gateway" className="text-xs">
            Gateway
          </Label>
        </div>
      </TooltipTrigger>
      <TooltipContent>
        {enabled
          ? "Turn off to publish each service on its own host port"
          : "Front every API and UI with one gateway"}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Save the diagram for use elsewhere: a graph snapshot (what an
 * `EnvironmentDiagram` embed renders) or a static SVG/PNG image.
 */
function ExportActions({
  graph,
  files,
  deploy,
  name,
  onError,
}: {
  graph: GraphDto | undefined;
  files: readonly string[] | undefined;
  deploy: readonly DeployStepDto[] | undefined;
  name: string;
  onError?: (message: string) => void;
}) {
  const renderImage = useDiagramImage();
  const empty = !graph || graph.nodes.length === 0;

  const exportSnapshot = () => {
    if (!graph) return;
    downloadText(
      stringifyGraphSnapshot(createGraphSnapshot(graph, { files, deploy })),
      `${name}${GRAPH_SNAPSHOT_EXTENSION}`,
    );
  };
  const exportImage = (format: DiagramImageFormat) => {
    renderImage({ format })
      .then((url) => downloadUrl(url, `${name}.${format}`))
      .catch((e: unknown) => {
        const reason = e instanceof Error ? e.message : String(e);
        onError?.(
          `Could not export the diagram as ${format.toUpperCase()}: ${reason}`,
        );
      });
  };

  return (
    <>
      <IconAction
        label="Export snapshot (embed in docs)"
        disabled={empty}
        onClick={exportSnapshot}
      >
        <FileJson className="h-4 w-4" />
      </IconAction>
      <IconAction
        label="Export SVG"
        disabled={empty}
        onClick={() => exportImage("svg")}
      >
        <FileImage className="h-4 w-4" />
      </IconAction>
      <IconAction
        label="Export PNG"
        disabled={empty}
        onClick={() => exportImage("png")}
      >
        <ImageDown className="h-4 w-4" />
      </IconAction>
    </>
  );
}

function IconAction({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            aria-label={label}
            disabled={disabled}
            onClick={onClick}
          >
            {children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom" className="text-xs">
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

export interface CanvasNoticeProps {
  /** The latest planning error; the canvas keeps the last good plan. */
  error: string | null;
  /** A transient, informational message. */
  notice: string | null;
  onDismissNotice: () => void;
}

/** Planning errors and transient notices, along the canvas's bottom edge. */
export function CanvasNotice({
  error,
  notice,
  onDismissNotice,
}: CanvasNoticeProps) {
  if (!error && !notice) return null;
  return (
    <Panel position="bottom-center">
      <div className="flex max-w-xl flex-col gap-2">
        {error && (
          <div className="flex items-start gap-2 rounded-lg border border-destructive/40 bg-card px-3 py-2 text-sm text-destructive shadow-sm">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="min-w-0 break-words">
              {error}
              <span className="text-muted-foreground">
                {" "}
                Showing the last environment that planned.
              </span>
            </span>
          </div>
        )}
        {notice && (
          <div className="flex items-start gap-2 rounded-lg border border-border bg-card px-3 py-2 text-sm text-card-foreground shadow-sm">
            <Info className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <span className="min-w-0 flex-1 break-words">{notice}</span>
            <button
              type="button"
              aria-label="Dismiss"
              className="text-muted-foreground hover:text-foreground"
              onClick={onDismissNotice}
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        )}
      </div>
    </Panel>
  );
}
