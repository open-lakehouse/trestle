// Where a viewer gets rendered files from. The editor holds every file of the
// plan in memory; an embedded diagram knows only the paths (its snapshot's
// manifest) and reads contents on demand from the host, typically out of the
// rendered environment shipped next to the snapshot.

import type { OutputFile } from "../types";

export interface FileSource {
  /** Every viewable path, relative to the rendered environment's root. */
  paths: readonly string[];
  /** A file's contents. Only paths listed in `paths` are ever read. */
  read: (path: string) => string | Promise<string>;
}

/** A source over files already in memory (a plan's `files`). */
export function fileSourceOf(files: readonly OutputFile[]): FileSource {
  const contents = new Map(files.map((f) => [f.path, f.contents]));
  return {
    paths: files.map((f) => f.path),
    read: (path) => {
      const found = contents.get(path);
      if (found === undefined) throw new Error(`no file at ${path}`);
      return found;
    },
  };
}

/** The directory a module's own files are rendered under. */
export function moduleDir(id: string): string {
  return `modules/${id}/`;
}

/**
 * The files rendered for a module: those under `modules/<id>/`, its
 * `compose.yaml` first, then by path.
 */
export function moduleFiles(paths: readonly string[], id: string): string[] {
  const dir = moduleDir(id);
  const compose = `${dir}compose.yaml`;
  return paths
    .filter((p) => p.startsWith(dir))
    .sort((a, b) =>
      a === compose ? -1 : b === compose ? 1 : a.localeCompare(b),
    );
}

/** A module file's path within the module's directory. */
export function relativePath(path: string, id: string): string {
  const dir = moduleDir(id);
  return path.startsWith(dir) ? path.slice(dir.length) : path;
}

/** Group paths by the module directory they sit in. */
export function filesByModule(paths: readonly string[]): Map<string, string[]> {
  const ids = new Set<string>();
  for (const p of paths) {
    const match = /^modules\/([^/]+)\//.exec(p);
    if (match?.[1]) ids.add(match[1]);
  }
  return new Map([...ids].map((id) => [id, moduleFiles(paths, id)]));
}
