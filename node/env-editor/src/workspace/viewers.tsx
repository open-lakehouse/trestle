// The shell's file and diff viewers: a host's (`EditorContributions.fileViewer`
// / `diffViewer`, e.g. Monaco), else the editor's own.

import type { ReactNode } from "react";
import { CodeViewer } from "../artifacts/CodeViewer.js";
import { DiffViewer } from "../artifacts/DiffViewer.js";
import {
  type DiffViewerProps,
  type FileViewerProps,
  useContributions,
} from "./contributions.js";

/** Renders one rendered file read-only. */
export function useFileViewer(): (props: FileViewerProps) => ReactNode {
  const host = useContributions().contributions?.fileViewer;
  return (
    host ??
    ((props) => (
      <CodeViewer
        key={props.path}
        path={props.path}
        read={props.read}
        header="compact"
      />
    ))
  );
}

/** Renders one file's change. */
export function useDiffViewer(): (props: DiffViewerProps) => ReactNode {
  const host = useContributions().contributions?.diffViewer;
  return (
    host ??
    ((props) => (
      <DiffViewer
        path={props.path}
        original={props.original}
        modified={props.modified}
      />
    ))
  );
}
