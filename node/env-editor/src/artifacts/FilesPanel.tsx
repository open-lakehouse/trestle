import { cn } from "@open-lakehouse/ui-kit";
import { ChevronRight, FileText, Folder } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { OutputFile, PlanResult } from "../types";
import {
  buildFileTree,
  defaultFilePath,
  type FileTreeNode,
} from "./buildFileTree";
import { CodeViewer } from "./CodeViewer";
import { fileSourceOf } from "./fileSource";

export interface FilesPanelProps {
  plan: PlanResult | null;
  /** The file to open first, if the plan has it (e.g. picked in the inspector). */
  initialPath?: string;
}

// A stable identity so the default-selection effect doesn't re-run every render
// while no plan is loaded.
const EMPTY_FILES: OutputFile[] = [];

function FileTreeItem({
  node,
  depth,
  selectedPath,
  onSelect,
}: {
  node: FileTreeNode;
  depth: number;
  selectedPath: string | null;
  onSelect: (path: string) => void;
}) {
  const isDir = Boolean(node.children?.length);
  const isSelected = node.file && selectedPath === node.path;
  const [open, setOpen] = useState(depth < 2);

  if (isDir) {
    return (
      <div>
        <button
          type="button"
          className="flex w-full items-center gap-1 rounded-sm px-2 py-1 text-left text-xs text-muted-foreground hover:bg-muted/60"
          style={{ paddingLeft: `${depth * 12 + 8}px` }}
          onClick={() => setOpen((v) => !v)}
        >
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 transition-transform",
              open && "rotate-90",
            )}
          />
          <Folder className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{node.name}</span>
        </button>
        {open &&
          node.children?.map((child) => (
            <FileTreeItem
              key={child.path}
              node={child}
              depth={depth + 1}
              selectedPath={selectedPath}
              onSelect={onSelect}
            />
          ))}
      </div>
    );
  }

  if (!node.file) return null;

  return (
    <button
      type="button"
      className={cn(
        "flex w-full items-center gap-1.5 rounded-sm px-2 py-1 text-left text-xs hover:bg-muted/60",
        isSelected ? "bg-primary/10 text-primary" : "text-card-foreground",
      )}
      style={{ paddingLeft: `${depth * 12 + 20}px` }}
      onClick={() => onSelect(node.path)}
    >
      <FileText className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate">{node.name}</span>
    </button>
  );
}

/**
 * Browse every non-sensitive file the planner materialized: tree on the left,
 * read-only content viewer on the right.
 */
export function FilesPanel({ plan, initialPath }: FilesPanelProps) {
  const files = plan?.files ?? EMPTY_FILES;
  const tree = useMemo(() => buildFileTree(files), [files]);
  const source = useMemo(() => fileSourceOf(files), [files]);
  const [selectedPath, setSelectedPath] = useState<string | null>(
    initialPath ?? null,
  );

  useEffect(() => {
    const next = defaultFilePath(files);
    setSelectedPath((current) =>
      current && files.some((f) => f.path === current) ? current : next,
    );
  }, [files]);

  const selectedFile =
    files.find((f) => f.path === selectedPath) ?? files[0] ?? null;

  if (!plan) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
        Add components to the environment to inspect the rendered files.
      </div>
    );
  }

  if (files.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
        The selected modules produced no viewable files.
      </div>
    );
  }

  return (
    <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[minmax(12rem,16rem)_minmax(0,1fr)]">
      <div className="min-h-0 overflow-auto border-b p-2 md:border-b-0 md:border-r">
        {tree.map((node) => (
          <FileTreeItem
            key={node.path}
            node={node}
            depth={0}
            selectedPath={selectedPath}
            onSelect={setSelectedPath}
          />
        ))}
      </div>
      <div className="flex min-h-[240px] flex-col">
        {selectedFile ? (
          <CodeViewer
            key={selectedFile.path}
            path={selectedFile.path}
            read={source.read}
          />
        ) : (
          <div className="flex h-full items-center justify-center p-6 text-sm text-muted-foreground">
            Select a file to preview its contents.
          </div>
        )}
      </div>
    </div>
  );
}
