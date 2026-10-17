import { cn } from "@open-lakehouse/ui-kit";
import { ExternalLink, FileCode2, X } from "lucide-react";
import {
  type CSSProperties,
  type KeyboardEvent,
  useEffect,
  useRef,
} from "react";
import { ModuleIcon } from "../graph/projectIcons";
import { roleStyle } from "../graph/roleTheme";
import type { GraphNodeDto } from "../types";
import { CodeViewer } from "./CodeViewer";
import { type FileSource, relativePath } from "./fileSource";

export interface NodeFilesDialogProps {
  /** The node whose files are shown; the dialog is closed without one. */
  node: GraphNodeDto | undefined;
  /** The node's files, in tab order. */
  paths: readonly string[];
  read: FileSource["read"];
  /** The open tab's path; defaults to the first file. */
  path?: string;
  onPathChange: (path: string) => void;
  onClose: () => void;
  /** Show the file in a full file explorer; without it there's no such button. */
  onOpenInExplorer?: (path: string) => void;
}

// The dialog element is only the top-layer frame (a native modal `<dialog>`
// draws above everything, an expanded diagram included); the overlay inside it
// is ordinary content, so the surrounding style scope applies.
const DIALOG_STYLE: CSSProperties = {
  position: "fixed",
  inset: 0,
  width: "100vw",
  height: "100vh",
  maxWidth: "none",
  maxHeight: "none",
  margin: 0,
  padding: 0,
  border: 0,
  background: "transparent",
  overflow: "hidden",
};

/**
 * A node's rendered config files in a modal: a tab per file over a
 * highlighted, read-only viewer. Shared by the editor and the embedded
 * diagram, so it is built on a native `<dialog>` rather than a UI library's.
 */
export function NodeFilesDialog({
  node,
  paths,
  read,
  path,
  onPathChange,
  onClose,
  onOpenInExplorer,
}: NodeFilesDialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const open = node !== undefined && paths.length > 0;
  const active = path && paths.includes(path) ? path : paths[0];

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      aria-label={node ? `${node.display_name ?? node.id} files` : "Files"}
      style={DIALOG_STYLE}
      // Escape fires `close`; keep the host's state in step with it.
      onClose={onClose}
    >
      {open && active && (
        // biome-ignore lint/a11y/noStaticElementInteractions: the click only catches the backdrop; Escape already closes a modal <dialog> natively.
        // biome-ignore lint/a11y/useKeyWithClickEvents: see above.
        <div
          className="flex h-full w-full items-center justify-center bg-black/60 p-4"
          onClick={(e) => {
            if (e.target === e.currentTarget) onClose();
          }}
        >
          <div className="flex h-[min(80vh,48rem)] w-[min(92vw,64rem)] flex-col overflow-hidden rounded-xl border border-border bg-card text-card-foreground shadow-2xl">
            <Header
              node={node}
              count={paths.length}
              onOpenInExplorer={
                onOpenInExplorer ? () => onOpenInExplorer(active) : undefined
              }
              onClose={onClose}
            />
            <FileTabs
              moduleId={node.id}
              paths={paths}
              active={active}
              onSelect={onPathChange}
            />
            <div
              role="tabpanel"
              aria-label={active}
              className="flex min-h-0 flex-1 flex-col"
            >
              <CodeViewer
                key={active}
                path={active}
                read={read}
                header="compact"
              />
            </div>
          </div>
        </div>
      )}
    </dialog>
  );
}

function Header({
  node,
  count,
  onOpenInExplorer,
  onClose,
}: {
  node: GraphNodeDto;
  count: number;
  onOpenInExplorer?: () => void;
  onClose: () => void;
}) {
  const style = roleStyle(node.role);
  return (
    <div className="flex shrink-0 items-center gap-3 border-b border-border px-4 py-3">
      <div
        className={cn(
          "flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-current/10",
          style.accent,
        )}
      >
        <ModuleIcon moduleId={node.id} role={node.role} className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold">
          {node.display_name ?? node.id}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {style.label} · {count} config {count === 1 ? "file" : "files"}
        </div>
      </div>
      {onOpenInExplorer && (
        <button
          type="button"
          onClick={onOpenInExplorer}
          className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          <ExternalLink className="h-3.5 w-3.5" />
          Open in Files
        </button>
      )}
      <button
        type="button"
        onClick={onClose}
        aria-label="Close"
        title="Close"
        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}

/** A tab per file; arrow keys, Home and End move between them. */
function FileTabs({
  moduleId,
  paths,
  active,
  onSelect,
}: {
  moduleId: string;
  paths: readonly string[];
  active: string;
  onSelect: (path: string) => void;
}) {
  const tabs = useRef<Map<string, HTMLButtonElement>>(new Map());

  const onKeyDown = (e: KeyboardEvent) => {
    const i = paths.indexOf(active);
    const next =
      e.key === "ArrowRight"
        ? paths[(i + 1) % paths.length]
        : e.key === "ArrowLeft"
          ? paths[(i - 1 + paths.length) % paths.length]
          : e.key === "Home"
            ? paths[0]
            : e.key === "End"
              ? paths[paths.length - 1]
              : undefined;
    if (!next) return;
    e.preventDefault();
    onSelect(next);
    tabs.current.get(next)?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label="Files"
      tabIndex={-1}
      className="flex shrink-0 gap-1 overflow-x-auto border-b border-border bg-muted/40 px-2 pt-1.5"
      onKeyDown={onKeyDown}
    >
      {paths.map((p) => {
        const selected = p === active;
        return (
          <button
            key={p}
            ref={(el) => {
              if (el) tabs.current.set(p, el);
              else tabs.current.delete(p);
            }}
            type="button"
            role="tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(p)}
            title={p}
            className={cn(
              "-mb-px inline-flex shrink-0 items-center gap-1.5 rounded-t-md border border-b-0 px-3 py-1.5 font-mono text-[11px] transition-colors",
              selected
                ? "border-border bg-card text-foreground"
                : "border-transparent text-muted-foreground hover:bg-muted hover:text-foreground",
            )}
          >
            <FileCode2 className="h-3.5 w-3.5 shrink-0" />
            {relativePath(p, moduleId)}
          </button>
        );
      })}
    </div>
  );
}
