import type { OutputFile } from "../types";

export interface FileTreeNode {
  name: string;
  path: string;
  children?: FileTreeNode[];
  file?: OutputFile;
}

function sortTree(nodes: FileTreeNode[]): void {
  nodes.sort((a, b) => {
    const aDir = Boolean(a.children);
    const bDir = Boolean(b.children);
    if (aDir !== bDir) return aDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  for (const node of nodes) {
    if (node.children) sortTree(node.children);
  }
}

/** Build a directory tree from a flat list of materialized file paths. */
export function buildFileTree(files: OutputFile[]): FileTreeNode[] {
  const root: FileTreeNode[] = [];

  for (const file of files) {
    const parts = file.path.split("/").filter(Boolean);
    let level = root;
    let currentPath = "";

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      const isFile = i === parts.length - 1;

      let node = level.find((n) => n.name === part);
      if (!node) {
        node = isFile
          ? { name: part, path: currentPath, file }
          : { name: part, path: currentPath, children: [] };
        level.push(node);
      } else if (isFile) {
        node.file = file;
      }

      if (!isFile) {
        node.children ??= [];
        level = node.children;
      }
    }
  }

  sortTree(root);
  return root;
}

/** Pick a sensible default file when the plan changes. */
export function defaultFilePath(files: OutputFile[]): string | null {
  if (files.length === 0) return null;
  return (
    files.find((f) => f.path === "compose.yaml")?.path ?? files[0]?.path ?? null
  );
}
