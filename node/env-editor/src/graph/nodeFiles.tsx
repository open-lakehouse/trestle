// The canvas's file seam: which rendered files belong to each node, and a way
// to open them. Read-only diagrams get it as much as the editor (a host that
// passes files to `MarkitectureCanvas` turns on a node's files chip). It
// travels by context rather than node `data`, so changing the files never
// re-runs the ELK layout.

import { createContext, useContext } from "react";

/** The node (and optionally the file) a files dialog shows. */
export interface NodeFilesTarget {
  id: string;
  path?: string;
}

export interface NodeFilesContextValue {
  /** A node's files, in tab order; empty when it has none. */
  filesOf: (id: string) => readonly string[];
  /** Open a node's files, at `path` if given. */
  open: (id: string, path?: string) => void;
}

export const NodeFilesContext = createContext<NodeFilesContextValue | null>(
  null,
);

/** A node's files and how to open them, or `null` when it has none. */
export function useNodeFiles(id: string): {
  paths: readonly string[];
  open: (path?: string) => void;
} | null {
  const ctx = useContext(NodeFilesContext);
  const paths = ctx?.filesOf(id) ?? [];
  if (!ctx || paths.length === 0) return null;
  return { paths, open: (path) => ctx.open(id, path) };
}
