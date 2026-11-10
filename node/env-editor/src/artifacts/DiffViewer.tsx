import { cn } from "@open-lakehouse/ui-kit";
import { useMemo } from "react";
import { foldContext, lineDiff } from "./lineDiff.js";

export interface DiffViewerProps {
  path: string;
  /** Absent for an added file. */
  original?: string;
  /** Absent for a removed file. */
  modified?: string;
  className?: string;
}

const TONE = {
  "+": "bg-[hsl(var(--status-done)/0.12)] text-[hsl(var(--status-done))]",
  "-": "bg-destructive/10 text-destructive",
  " ": "text-foreground",
};

/** One file's change as a unified line diff: the editor's default, which a
 *  host may replace (`EditorContributions.diffViewer`). */
export function DiffViewer({
  path,
  original,
  modified,
  className,
}: DiffViewerProps) {
  const rows = useMemo(
    () => foldContext(lineDiff(original, modified)),
    [original, modified],
  );
  return (
    <figure
      aria-label={`Changes to ${path}`}
      className={cn("overflow-x-auto font-mono text-xs leading-5", className)}
    >
      {rows.map((row, i) =>
        row.op === "fold" ? (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity but their place.
            key={i}
            className="px-3 text-muted-foreground"
          >
            ⋯ {row.count} unchanged line{row.count === 1 ? "" : "s"}
          </div>
        ) : (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: rows have no identity but their place.
            key={i}
            className={cn("whitespace-pre px-3", TONE[row.op])}
          >
            <span className="mr-2 select-none opacity-70">{row.op}</span>
            {row.text}
          </div>
        ),
      )}
    </figure>
  );
}
