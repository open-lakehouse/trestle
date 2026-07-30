import { Button, cn } from "@open-lakehouse/ui-kit";
import { Check, ChevronRight, Copy, FileText, Folder } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { OutputFile, PlanResult } from "../types";
import {
  buildFileTree,
  defaultFilePath,
  type FileTreeNode,
} from "./buildFileTree";

export interface FilesPanelProps {
  plan: PlanResult | null;
}

// A stable identity so the default-selection effect doesn't re-run every render
// while no plan is loaded.
const EMPTY_FILES: OutputFile[] = [];

function CodeViewer({ file }: { file: OutputFile }) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    void navigator.clipboard.writeText(file.contents).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-2 border-b px-3 py-2">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium text-card-foreground">
            {file.path.split("/").pop()}
          </div>
          <div className="truncate font-mono text-[11px] text-muted-foreground">
            {file.path}
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={copy}
          aria-label="Copy file contents"
        >
          {copied ? (
            <Check className="h-4 w-4" />
          ) : (
            <Copy className="h-4 w-4" />
          )}
        </Button>
      </div>
      <pre className="min-h-0 flex-1 overflow-auto bg-muted/20 p-3 font-mono text-xs leading-relaxed text-foreground">
        <code>{file.contents}</code>
      </pre>
    </div>
  );
}

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
export function FilesPanel({ plan }: FilesPanelProps) {
  const files = plan?.files ?? EMPTY_FILES;
  const tree = useMemo(() => buildFileTree(files), [files]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

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
        Generate an environment to inspect the rendered files.
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
          <CodeViewer file={selectedFile} />
        ) : (
          <div className="flex h-full items-center justify-center p-6 text-sm text-muted-foreground">
            Select a file to preview its contents.
          </div>
        )}
      </div>
    </div>
  );
}
