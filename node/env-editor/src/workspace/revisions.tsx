// The shell's revision views, shown only when the store keeps revisions: the
// sidebar's history of an environment (commit its draft, open or compare a
// revision), a read-only revision editor (its diagram and what it selects), and
// a comparison of two revisions or of a revision and the draft, and an
// environment's Changes (its draft against its last revision). Comparisons
// list what the selection changed, then each rendered file that differs,
// through the host's diff viewer where it gives one.

import { Badge, Button, Input } from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import { GitCommitHorizontal, GitCompare, Loader2, Undo2 } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { EnvironmentDiagram } from "../diagram/EnvironmentDiagram.js";
import { errorMessage } from "../errors.js";
import { usePlanner } from "../planner.js";
import type { CatalogDto, PlanResult, Selection } from "../types.js";
import { Breadcrumb, SidebarHeader } from "./chrome.js";
import {
  describeHosted,
  describeHostedChange,
  diffFiles,
  diffSelections,
  type FileChange,
  isEmptyDiff,
  type SelectionDiff,
} from "./diff.js";
import type { EditorTab } from "./editors.js";
import { contextFor, type EnvDoc, type WorkspaceDoc } from "./model.js";
import {
  type RevisionDoc,
  type RevisionSummary,
  selectionDigest,
} from "./store.js";
import { useWorkspace } from "./useWorkspace.js";
import { useDiffViewer } from "./viewers.js";

/** A pending, loaded or failed async value. */
type Loaded<T> = { value: T } | { error: string } | undefined;

/** Load `load()` whenever `deps` change; stale results are dropped. */
function useLoaded<T>(load: () => Promise<T>, deps: readonly unknown[]) {
  const [state, setState] = useState<Loaded<T>>();
  useEffect(() => {
    let cancelled = false;
    setState(undefined);
    load().then(
      (value) => !cancelled && setState({ value }),
      (e: unknown) => !cancelled && setState({ error: errorMessage(e) }),
    );
    return () => {
      cancelled = true;
    };
    // biome-ignore lint/correctness/useExhaustiveDependencies: the caller passes `load`'s dependencies
  }, deps);
  return state;
}

function Pending({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full items-center justify-center gap-2 p-6 text-sm text-muted-foreground">
      {children}
    </div>
  );
}

const short = (digest: string) => digest.slice(0, 8);

// ---------------------------------------------------------------------------
// Sidebar: an environment's history
// ---------------------------------------------------------------------------

export interface RevisionsViewProps {
  workspace: WorkspaceDoc;
  /** The environment whose history to show (the active one). */
  env: EnvDoc | undefined;
  catalog: CatalogDto | undefined;
  onOpen: (tab: EditorTab) => void;
  /** Focuses the commit message whenever it changes (the Commit command). */
  focusCommit?: number;
}

/** The sidebar's history of `env`: its draft, then its revisions. */
export function RevisionsView({
  workspace,
  env,
  catalog,
  onOpen,
  focusCommit,
}: RevisionsViewProps) {
  const ws = useWorkspace();
  const revisions = ws.revisions;
  const list = useLoaded(
    async () => (env && revisions ? revisions.list(env.id) : []),
    [env?.id, revisions?.version],
  );
  const draft = env?.selection ?? catalog?.default_selection;
  const draftDigest = useLoaded(
    async () => (draft ? selectionDigest(draft) : undefined),
    [draft],
  );
  const [message, setMessage] = useState("");
  const messageRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focusCommit) messageRef.current?.focus();
  }, [focusCommit]);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  if (!revisions) return null;
  const items = list && "value" in list ? list.value : [];
  const head = items[0];
  const unchanged =
    !!head &&
    !!draftDigest &&
    "value" in draftDigest &&
    draftDigest.value === head.digest;

  const commit = () => {
    if (!env) return;
    setBusy(true);
    setProblem(null);
    revisions
      .commit(env.id, message.trim() || undefined)
      .then(() => setMessage(""))
      .catch((e: unknown) => setProblem(errorMessage(e)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SidebarHeader title={env ? `History · ${env.name}` : "History"} />
      {!env ? (
        <p className="px-3 text-xs text-muted-foreground">
          Open an environment to see its revisions.
        </p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3 text-sm">
          <section className="mb-3 space-y-2 rounded-md border p-2">
            <div className="flex items-center justify-between">
              <span className="font-medium">Draft</span>
              {head && (
                <Badge variant={unchanged ? "default" : "primary"}>
                  {unchanged ? `same as #${head.number}` : "changed"}
                </Badge>
              )}
            </div>
            {!env.selection && (
              <p className="text-xs text-muted-foreground">
                Edit the environment to commit it.
              </p>
            )}
            <Input
              ref={messageRef}
              aria-label="Revision message"
              placeholder="Message (optional)"
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !busy && env.selection) commit();
              }}
            />
            <div className="flex gap-2">
              <Button
                type="button"
                size="sm"
                disabled={busy || !env.selection || unchanged}
                onClick={commit}
              >
                {busy ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <GitCommitHorizontal className="h-4 w-4" />
                )}
                Commit
              </Button>
              {head && (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={unchanged}
                  onClick={() =>
                    onOpen({
                      env: env.id,
                      compare: { from: head.number, to: "draft" },
                    })
                  }
                >
                  <GitCompare className="h-4 w-4" />
                  Diff
                </Button>
              )}
            </div>
            {problem && <p className="text-xs text-destructive">{problem}</p>}
          </section>

          {list === undefined && (
            <span className="text-xs text-muted-foreground">Loading…</span>
          )}
          {list && "error" in list && (
            <span className="text-xs text-destructive">{list.error}</span>
          )}
          {list && "value" in list && items.length === 0 && (
            <span className="text-xs text-muted-foreground">
              No revisions yet.
            </span>
          )}
          <ol aria-label={`Revisions of ${env.name}`} className="space-y-1">
            {items.map((r, i) => (
              <RevisionRow
                key={r.number}
                revision={r}
                previous={items[i + 1]}
                onOpen={() => onOpen({ env: env.id, revision: r.number })}
                onCompare={(from) =>
                  onOpen({ env: env.id, compare: { from, to: r.number } })
                }
              />
            ))}
          </ol>
          <p className="mt-3 text-[11px] text-muted-foreground">
            Workspace <code>{workspace.name}</code>; revisions are immutable.
          </p>
        </div>
      )}
    </div>
  );
}

function RevisionRow({
  revision,
  previous,
  onOpen,
  onCompare,
}: {
  revision: RevisionSummary;
  previous: RevisionSummary | undefined;
  onOpen: () => void;
  onCompare: (from: number) => void;
}) {
  return (
    <li className="group flex items-start justify-between gap-2 rounded px-1 py-1 hover:bg-sidebar-accent">
      <button
        type="button"
        className="min-w-0 flex-1 text-left"
        onClick={onOpen}
        title={revision.digest}
      >
        <span className="font-medium">#{revision.number}</span>{" "}
        <span className="font-mono text-[11px] text-muted-foreground">
          {short(revision.digest)}
        </span>
        {revision.message && (
          <span className="block truncate text-xs">{revision.message}</span>
        )}
        {revision.createdAt && (
          <span className="block text-[11px] text-muted-foreground">
            {new Date(revision.createdAt).toLocaleString()}
          </span>
        )}
      </button>
      {previous && (
        <button
          type="button"
          aria-label={`Compare #${previous.number} with #${revision.number}`}
          title={`Compare with #${previous.number}`}
          className="invisible rounded p-0.5 text-muted-foreground hover:text-foreground group-hover:visible"
          onClick={() => onCompare(previous.number)}
        >
          <GitCompare className="h-3.5 w-3.5" />
        </button>
      )}
    </li>
  );
}

// ---------------------------------------------------------------------------
// Editor: one revision, read-only
// ---------------------------------------------------------------------------

export interface RevisionEditorProps {
  workspace: WorkspaceDoc;
  env: EnvDoc;
  number: number;
  colorMode?: ColorMode;
}

/** Revision `number` of `env`, read-only: its diagram and what it selects. */
export function RevisionEditor({
  workspace,
  env,
  number,
  colorMode,
}: RevisionEditorProps) {
  const ws = useWorkspace();
  const planner = usePlanner();
  const revision = useLoaded(async () => {
    const doc = await ws.revisions?.load(env.id, number);
    if (!doc) throw new Error(`${env.name} has no revision #${number}`);
    return doc;
  }, [env.id, number, ws.revisions]);
  const selection =
    revision && "value" in revision ? revision.value.selection : undefined;
  const plan = useLoaded(
    async (): Promise<PlanResult | undefined> =>
      selection &&
      planner.plan(selection, undefined, contextFor(workspace, env)),
    [selection, planner, workspace, env],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between gap-2 border-b px-3">
        <Breadcrumb parts={[workspace.name, env.name, `Revision #${number}`]} />
        {revision && "value" in revision && (
          <div className="flex items-center gap-2">
            <Badge variant="outline">read-only</Badge>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => ws.setSelection(env.id, revision.value.selection)}
              title="Replace the draft with this revision's selection"
            >
              <Undo2 className="h-4 w-4" />
              Restore to draft
            </Button>
          </div>
        )}
      </div>
      {revision === undefined ? (
        <Pending>
          <Loader2 className="h-4 w-4 animate-spin" /> Loading revision…
        </Pending>
      ) : "error" in revision ? (
        <Pending>{revision.error}</Pending>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="min-w-0 flex-1">
            {plan === undefined ? (
              <Pending>
                <Loader2 className="h-4 w-4 animate-spin" /> Planning…
              </Pending>
            ) : "error" in plan ? (
              <Pending>This revision doesn't plan: {plan.error}</Pending>
            ) : plan.value ? (
              <EnvironmentDiagram
                graph={plan.value.graph}
                colorMode={colorMode}
              />
            ) : null}
          </div>
          <SelectionSummary revision={revision.value} />
        </div>
      )}
    </div>
  );
}

function SelectionSummary({ revision }: { revision: RevisionDoc }) {
  const { selection } = revision;
  const knobs = Object.entries(selection.knob_overrides).flatMap(
    ([module, values]) =>
      Object.entries(values).map(([knob, value]) => ({ module, knob, value })),
  );
  const hosted = Object.entries(selection.hosted ?? {}).sort(([a], [b]) =>
    a < b ? -1 : a > b ? 1 : 0,
  );
  return (
    <aside className="w-64 shrink-0 overflow-y-auto border-l p-3 text-sm">
      <Section title="Revision">
        <div>#{revision.number}</div>
        <div className="font-mono text-[11px] text-muted-foreground">
          {revision.digest}
        </div>
        {revision.message && <div className="mt-1">{revision.message}</div>}
      </Section>
      <Section title="Modules">
        <IdList ids={selection.modules} />
      </Section>
      {selection.capabilities.length > 0 && (
        <Section title="Capabilities">
          <IdList ids={selection.capabilities} />
        </Section>
      )}
      {hosted.length > 0 && (
        <Section title="Hosted">
          <ul className="space-y-0.5">
            {hosted.map(([module, resource]) => (
              <li key={module} className="font-mono text-[11px]">
                {module}: {describeHosted(resource)}
              </li>
            ))}
          </ul>
        </Section>
      )}
      {knobs.length > 0 && (
        <Section title="Settings">
          <ul className="space-y-0.5">
            {knobs.map(({ module, knob, value }) => (
              <li key={`${module}.${knob}`} className="font-mono text-[11px]">
                {module}.{knob} = {value}
              </li>
            ))}
          </ul>
        </Section>
      )}
    </aside>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mb-3">
      <h3 className="mb-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      {children}
    </section>
  );
}

function IdList({ ids }: { ids: readonly string[] }) {
  if (ids.length === 0) {
    return <span className="text-xs text-muted-foreground">none</span>;
  }
  return (
    <ul className="space-y-0.5">
      {ids.map((id) => (
        <li key={id} className="font-mono text-xs">
          {id}
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------------------
// Editor: comparing two revisions
// ---------------------------------------------------------------------------

export interface CompareEditorProps {
  workspace: WorkspaceDoc;
  env: EnvDoc;
  from: number;
  to: number | "draft";
  catalog: CatalogDto | undefined;
}

/** What changed from revision `from` to `to` (a revision, or the draft). */
export function CompareEditor({
  workspace,
  env,
  from,
  to,
  catalog,
}: CompareEditorProps) {
  const label = (n: number | "draft") => (n === "draft" ? "draft" : `#${n}`);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center border-b px-3">
        <Breadcrumb
          parts={[workspace.name, env.name, `${label(from)} → ${label(to)}`]}
        />
      </div>
      <Comparison
        workspace={workspace}
        env={env}
        from={from}
        to={to}
        catalog={catalog}
      />
    </div>
  );
}

export interface ChangesViewProps {
  workspace: WorkspaceDoc;
  env: EnvDoc;
  catalog: CatalogDto | undefined;
}

/** An environment's Changes: its draft against its last revision. */
export function ChangesView({ workspace, env, catalog }: ChangesViewProps) {
  if (env.head === undefined) {
    return (
      <Pending>
        {env.name} has no revision yet: commit one to compare the draft against.
      </Pending>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <Comparison
        workspace={workspace}
        env={env}
        from={env.head}
        to="draft"
        catalog={catalog}
      />
    </div>
  );
}

/** Both sides of a comparison: what the selection changed, and the rendered
 *  files that differ. */
interface ComparisonResult {
  selection: SelectionDiff;
  files: FileChange[];
}

/** Revision `from` against `to` (a revision or the draft), both planned in
 *  the environment's context. */
function Comparison({ workspace, env, from, to, catalog }: CompareEditorProps) {
  const ws = useWorkspace();
  const planner = usePlanner();
  const draft = env.selection ?? catalog?.default_selection;
  const result = useLoaded(async (): Promise<ComparisonResult> => {
    const load = async (n: number): Promise<Selection> => {
      const doc = await ws.revisions?.load(env.id, n);
      if (!doc) throw new Error(`${env.name} has no revision #${n}`);
      return doc.selection;
    };
    const before = await load(from);
    const after = to === "draft" ? draft : await load(to);
    if (!after) throw new Error("The catalog is still loading.");
    const context = contextFor(workspace, env);
    const [was, now] = await Promise.all([
      planner.plan(before, undefined, context),
      planner.plan(after, undefined, context),
    ]);
    return {
      selection: diffSelections(before, after),
      files: diffFiles(was.files, now.files),
    };
  }, [env.id, from, to, draft, ws.revisions, planner, workspace]);
  const label = (n: number | "draft") => (n === "draft" ? "draft" : `#${n}`);

  if (result === undefined) {
    return (
      <Pending>
        <Loader2 className="h-4 w-4 animate-spin" /> Comparing…
      </Pending>
    );
  }
  if ("error" in result) return <Pending>{result.error}</Pending>;
  const { selection, files } = result.value;
  if (isEmptyDiff(selection) && files.length === 0) {
    return (
      <Pending>
        {label(from)} and {label(to)} select the same environment.
      </Pending>
    );
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      {!isEmptyDiff(selection) && <DiffList diff={selection} />}
      <FileChanges envId={env.id} files={files} />
    </div>
  );
}

/** Each rendered file that differs, through the diff viewer. */
function FileChanges({
  envId,
  files,
}: {
  envId: string;
  files: readonly FileChange[];
}) {
  const viewer = useDiffViewer();
  if (files.length === 0) return null;
  return (
    <section aria-label="Changed files" className="border-t">
      <h3 className="px-4 pt-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Files ({files.length})
      </h3>
      {files.map((file) => (
        <div key={file.path} className="mt-2 border-y">
          <div className="flex items-center gap-2 bg-muted/40 px-3 py-1 font-mono text-xs">
            <span className="truncate">{file.path}</span>
            <Badge variant="outline" className="text-[10px]">
              {file.original === undefined
                ? "added"
                : file.modified === undefined
                  ? "removed"
                  : "changed"}
            </Badge>
          </div>
          {viewer({
            envId,
            path: file.path,
            original: file.original,
            modified: file.modified,
          })}
        </div>
      ))}
    </section>
  );
}

function DiffList({ diff }: { diff: SelectionDiff }) {
  const rows: { sign: "+" | "-" | "~"; text: string }[] = [
    ...diff.modules.added.map((id) => ({
      sign: "+" as const,
      text: `module ${id}`,
    })),
    ...diff.modules.removed.map((id) => ({
      sign: "-" as const,
      text: `module ${id}`,
    })),
    ...diff.capabilities.added.map((id) => ({
      sign: "+" as const,
      text: `capability ${id}`,
    })),
    ...diff.capabilities.removed.map((id) => ({
      sign: "-" as const,
      text: `capability ${id}`,
    })),
    ...diff.knobs.map((k) => ({
      sign: "~" as const,
      text: `${k.module}.${k.knob}: ${k.from ?? "(default)"} → ${k.to ?? "(default)"}`,
    })),
    ...diff.hosted.map((c) => ({
      sign: "~" as const,
      text: describeHostedChange(c),
    })),
    ...diff.settings.map((field) => ({
      sign: "~" as const,
      text: `${field} changed`,
    })),
  ];
  const tone = {
    "+": "text-[hsl(var(--status-done))]",
    "-": "text-destructive",
    "~": "text-[hsl(var(--status-ready))]",
  };
  return (
    <ul aria-label="Changes" className="space-y-1 p-4 font-mono text-sm">
      {rows.map((row) => (
        <li key={`${row.sign}${row.text}`} className="flex gap-2">
          <span className={tone[row.sign]}>{row.sign}</span>
          <span>{row.text}</span>
        </li>
      ))}
    </ul>
  );
}
