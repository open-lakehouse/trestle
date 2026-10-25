import {
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
  Label,
  Switch,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Panel } from "@xyflow/react";
import {
  AlertTriangle,
  ChevronDown,
  ClipboardCopy,
  Download,
  FileArchive,
  FileCode,
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
  /** The plan's Mermaid diagram, for the Markdown export actions and the
   *  snapshot. */
  mermaid?: string;
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
  /**
   * `"floating"` (the default) is a card over the canvas's top-left corner,
   * rendered as a child of `MarkitectureCanvas`. `"bar"` is a flat,
   * full-width editor toolbar for above the canvas (an IDE's editor actions),
   * rendered under the same `ReactFlowProvider` as the canvas.
   */
  variant?: "floating" | "bar";
  /** Shown at the bar's start, before the actions (e.g. a breadcrumb). */
  leading?: ReactNode;
}

const MOD_KEY =
  typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform)
    ? "⌘"
    : "Ctrl+";

/**
 * The canvas's actions: add a component, undo/redo, export the diagram,
 * download the environment, and the planner's status. Floating, render it as a
 * child of `MarkitectureCanvas`; as a `"bar"`, above it (see `variant`).
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
  mermaid,
  exportName = "environment",
  onExportError,
  onDownloadBundle,
  bundling = false,
  bundleLabel = "Download environment (.zip)",
  gatewayEnabled = true,
  onToggleGateway,
  variant = "floating",
  leading,
}: CanvasToolbarProps) {
  const bar = variant === "bar";
  const actions = (
    <div
      role="toolbar"
      aria-label="Diagram"
      className={cn(
        "flex items-center gap-1",
        bar
          ? "h-10 shrink-0 border-b bg-background px-2"
          : "rounded-lg border border-border bg-card/95 p-1 shadow-sm",
      )}
    >
      {leading && (
        <>
          <div className="min-w-0 shrink truncate px-1">{leading}</div>
          <div className="flex-1" />
        </>
      )}
      <Button
        type="button"
        size="sm"
        className={cn("gap-1.5", bar ? "h-7" : "h-8")}
        onClick={onAdd}
      >
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
      <ExportMenu
        graph={graph}
        files={files}
        deploy={deploy}
        mermaid={mermaid}
        name={exportName}
        onError={onExportError}
        onDownloadBundle={onDownloadBundle}
        bundleDisabled={
          hasError || planning || bundling || componentCount === 0
        }
        bundling={bundling}
        bundleLabel={bundleLabel}
        compact={bar}
      />
      <div className="mx-1 h-5 w-px bg-border" />
      {onToggleGateway && (
        <>
          <GatewayToggle enabled={gatewayEnabled} onToggle={onToggleGateway} />
          <div className="mx-1 h-5 w-px bg-border" />
        </>
      )}
      <span
        className={cn(
          "inline-flex items-center gap-1.5 px-2 text-xs",
          hasError && !planning ? "text-destructive" : "text-muted-foreground",
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
  );
  return bar ? actions : <Panel position="top-left">{actions}</Panel>;
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
 * Everything the diagram and its environment save as, behind one Export menu:
 * a static image (SVG, PNG), Mermaid for Markdown (copied or as `.mmd`), a
 * graph snapshot (what an `EnvironmentDiagram` embed renders), and the whole
 * rendered environment as a zip.
 */
function ExportMenu({
  graph,
  files,
  deploy,
  mermaid,
  name,
  onError,
  onDownloadBundle,
  bundleDisabled,
  bundling,
  bundleLabel,
  compact,
}: {
  graph: GraphDto | undefined;
  files: readonly string[] | undefined;
  deploy: readonly DeployStepDto[] | undefined;
  mermaid: string | undefined;
  name: string;
  onError?: (message: string) => void;
  onDownloadBundle?: () => void;
  bundleDisabled: boolean;
  bundling: boolean;
  bundleLabel: string;
  /** Size the trigger for the flat editor bar. */
  compact: boolean;
}) {
  const renderImage = useDiagramImage();
  const empty = !graph || graph.nodes.length === 0;

  const exportSnapshot = () => {
    if (!graph) return;
    downloadText(
      stringifyGraphSnapshot(
        createGraphSnapshot(graph, { files, deploy, mermaid }),
      ),
      `${name}${GRAPH_SNAPSHOT_EXTENSION}`,
    );
  };
  const copyMermaid = () => {
    if (!mermaid) return;
    // `navigator.clipboard` is absent outside a secure context; report that too.
    Promise.resolve()
      .then(() =>
        navigator.clipboard.writeText(`\`\`\`mermaid\n${mermaid}\`\`\`\n`),
      )
      .catch((e: unknown) => {
        const reason = e instanceof Error ? e.message : String(e);
        onError?.(`Could not copy the Mermaid diagram: ${reason}`);
      });
  };
  const exportMermaid = () => {
    if (!mermaid) return;
    downloadText(mermaid, `${name}.mmd`, "text/vnd.mermaid");
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
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={cn("gap-1.5 px-2", compact ? "h-7" : "h-8")}
          aria-label="Export"
        >
          {bundling ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Download className="h-4 w-4" />
          )}
          Export
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel>Diagram image</DropdownMenuLabel>
        <DropdownMenuItem disabled={empty} onSelect={() => exportImage("svg")}>
          <FileImage />
          Image
          <DropdownMenuShortcut>.svg</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem disabled={empty} onSelect={() => exportImage("png")}>
          <ImageDown />
          Image
          <DropdownMenuShortcut>.png</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Docs</DropdownMenuLabel>
        <DropdownMenuItem disabled={empty || !mermaid} onSelect={copyMermaid}>
          <ClipboardCopy />
          Copy Mermaid
        </DropdownMenuItem>
        <DropdownMenuItem disabled={empty || !mermaid} onSelect={exportMermaid}>
          <FileCode />
          Mermaid
          <DropdownMenuShortcut>.mmd</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem disabled={empty} onSelect={exportSnapshot}>
          <FileJson />
          Snapshot for embeds
          <DropdownMenuShortcut>.json</DropdownMenuShortcut>
        </DropdownMenuItem>
        {onDownloadBundle && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Environment</DropdownMenuLabel>
            <DropdownMenuItem
              disabled={bundleDisabled}
              onSelect={onDownloadBundle}
              title={bundleLabel}
            >
              <FileArchive />
              Rendered files
              <DropdownMenuShortcut>.zip</DropdownMenuShortcut>
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
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
