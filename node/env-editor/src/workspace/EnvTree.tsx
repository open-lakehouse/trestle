// The explorer: the workspace's environments as a tree. Each environment
// expands into its diagram and the files it renders; picking one opens it in
// an editor tab. Environments are added, renamed, duplicated and removed here.

import { cn } from "@open-lakehouse/ui-kit";
import {
  ChevronRight,
  Copy,
  Download,
  FileText,
  Folder,
  FolderOpen,
  Layers,
  Loader2,
  Network,
  Pencil,
  Plus,
  Trash2,
  Workflow,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { buildFileTree, type FileTreeNode } from "../artifacts/buildFileTree";
import { EnvStatus, IconButton, NameInput, SidebarHeader } from "./chrome";
import type { EditorTab } from "./editors";
import {
  type EnvDoc,
  freeName,
  type PortConflict,
  SLOT_PORT_STRIDE,
  type WorkspaceDoc,
} from "./model";
import type { MemberPlan } from "./useMemberPlans";
import { useWorkspace } from "./useWorkspace";

/** What the explorer is asking a name for. */
type NameEdit =
  | { kind: "add" }
  | { kind: "rename"; id: string }
  | { kind: "duplicate"; id: string };

/** How far each tree level indents, in pixels. */
const INDENT = 12;

export interface EnvTreeProps {
  workspace: WorkspaceDoc;
  plans: Record<string, MemberPlan>;
  conflicts: PortConflict[];
  /** The active editor, highlighted in the tree. */
  active: EditorTab | undefined;
  onOpen: (tab: EditorTab) => void;
  onDownload?: () => void;
  downloading: boolean;
}

export function EnvTree({
  workspace,
  plans,
  conflicts,
  active,
  onOpen,
  onDownload,
  downloading,
}: EnvTreeProps) {
  const ws = useWorkspace();
  const [edit, setEdit] = useState<NameEdit | null>(null);
  // Expanded environments and folders, by env id / `<env id>:<path>`. The
  // active environment starts expanded.
  const [expanded, setExpanded] = useState<Set<string>>(
    () =>
      new Set(
        active ? [active.env] : workspace.envs.slice(0, 1).map((e) => e.id),
      ),
  );
  const toggle = (key: string, open?: boolean) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (open ?? !next.has(key)) next.add(key);
      else next.delete(key);
      return next;
    });
  const clashing = new Set(
    conflicts.flatMap((c) => [c.first.env, c.second.env]),
  );

  const submit = (name: string): string | undefined => {
    if (!edit) return undefined;
    if (edit.kind === "rename") return ws.renameEnv(edit.id, name);
    const env =
      edit.kind === "add" ? ws.addEnv(name) : ws.duplicateEnv(edit.id, name);
    if (typeof env === "string") return env;
    toggle(env.id, true);
    onOpen({ env: env.id });
    return undefined;
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SidebarHeader title="Environments">
        <IconButton
          label="New environment"
          onClick={() => setEdit({ kind: "add" })}
        >
          <Plus />
        </IconButton>
        {onDownload && (
          <IconButton
            label="Download workspace (.zip) for `trestle workspace`"
            onClick={onDownload}
            disabled={downloading}
          >
            {downloading ? <Loader2 className="animate-spin" /> : <Download />}
          </IconButton>
        )}
      </SidebarHeader>
      <div className="flex items-center gap-1.5 px-3 pb-2 text-xs text-muted-foreground">
        <Layers className="h-3.5 w-3.5" />
        <span className="truncate font-medium text-foreground">
          {workspace.name}
        </span>
        {workspace.mesh && (
          <span className="inline-flex items-center gap-1">
            · <Network className="h-3 w-3" /> mesh
          </span>
        )}
      </div>
      <div
        role="tree"
        aria-label="Environments"
        className="min-h-0 flex-1 overflow-y-auto pb-2"
      >
        {workspace.envs.map((env) =>
          edit?.kind === "rename" && edit.id === env.id ? (
            <div key={env.id} className="px-2 py-0.5">
              <NameInput
                initial={env.name}
                onSubmit={submit}
                onDone={() => setEdit(null)}
              />
            </div>
          ) : (
            <EnvNode
              key={env.id}
              env={env}
              plan={plans[env.id]}
              clashing={clashing.has(env.name)}
              expanded={expanded}
              onToggle={toggle}
              active={active}
              onOpen={onOpen}
              onRename={() => setEdit({ kind: "rename", id: env.id })}
              onDuplicate={() => setEdit({ kind: "duplicate", id: env.id })}
              onRemove={() => {
                if (
                  window.confirm(
                    `Remove \`${env.name}\` from the workspace? Its selection is lost.`,
                  )
                ) {
                  ws.removeEnv(env.id);
                }
              }}
            />
          ),
        )}
        {edit && edit.kind !== "rename" && (
          <div className="px-2 py-0.5">
            <NameInput
              initial={freeName(
                workspace,
                edit.kind === "add"
                  ? workspace.envs.length === 0
                    ? "dev"
                    : "prod"
                  : `${workspace.envs.find((e) => e.id === edit.id)?.name ?? "env"}-copy`,
              )}
              onSubmit={submit}
              onDone={() => setEdit(null)}
            />
          </div>
        )}
      </div>
    </div>
  );
}

/** One tree row: indented, with an optional disclosure chevron. */
function Row({
  depth,
  selected,
  disclosure,
  onClick,
  children,
  actions,
  title,
}: {
  depth: number;
  selected?: boolean;
  /** `true`/`false` draws an open/closed chevron; absent, a spacer. */
  disclosure?: boolean;
  onClick: () => void;
  children: ReactNode;
  actions?: ReactNode;
  title?: string;
}) {
  return (
    <div
      role="treeitem"
      aria-selected={!!selected}
      aria-expanded={disclosure}
      tabIndex={-1}
      className={cn(
        "group flex h-6 items-center pr-2 text-[13px]",
        selected ? "bg-sidebar-accent" : "hover:bg-sidebar-accent/60",
      )}
      style={{ paddingLeft: depth * INDENT + 6 }}
      title={title}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
      >
        {disclosure === undefined ? (
          <span className="w-3 shrink-0" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground transition-transform",
              disclosure && "rotate-90",
            )}
          />
        )}
        {children}
      </button>
      {actions && (
        <div className="hidden shrink-0 items-center group-hover:flex">
          {actions}
        </div>
      )}
    </div>
  );
}

function EnvNode({
  env,
  plan,
  clashing,
  expanded,
  onToggle,
  active,
  onOpen,
  onRename,
  onDuplicate,
  onRemove,
}: {
  env: EnvDoc;
  plan: MemberPlan | undefined;
  clashing: boolean;
  expanded: Set<string>;
  onToggle: (key: string, open?: boolean) => void;
  active: EditorTab | undefined;
  onOpen: (tab: EditorTab) => void;
  onRename: () => void;
  onDuplicate: () => void;
  onRemove: () => void;
}) {
  const open = expanded.has(env.id);
  const files = plan?.plan?.files;
  const tree = useMemo(() => (files ? buildFileTree(files) : []), [files]);
  const port = plan?.plan?.gateway.listeners[0]?.host_port;
  const here = active?.env === env.id;
  return (
    <>
      <Row
        depth={0}
        disclosure={open}
        onClick={() => onToggle(env.id)}
        title={`slot ${env.slot}: host ports +${env.slot * SLOT_PORT_STRIDE}`}
        actions={
          <>
            <IconButton label="Rename" onClick={onRename}>
              <Pencil />
            </IconButton>
            <IconButton
              label="Duplicate (e.g. dev → prod)"
              onClick={onDuplicate}
            >
              <Copy />
            </IconButton>
            <IconButton label="Remove" onClick={onRemove}>
              <Trash2 />
            </IconButton>
          </>
        }
      >
        <EnvStatus plan={plan} clashing={clashing} />
        <span className={cn("truncate", here && "font-medium")}>
          {env.name}
        </span>
        <span className="ml-auto shrink-0 font-mono text-[11px] text-muted-foreground group-hover:hidden">
          {port ? `:${port}` : `+${env.slot * SLOT_PORT_STRIDE}`}
        </span>
      </Row>
      {open && (
        <div>
          <Row
            depth={1}
            selected={here && active?.path === undefined}
            onClick={() => onOpen({ env: env.id })}
          >
            <Workflow className="h-3.5 w-3.5 shrink-0 text-primary" />
            <span className="truncate">Diagram</span>
          </Row>
          {plan === undefined && (
            <div
              className="flex h-6 items-center gap-1.5 text-xs text-muted-foreground"
              style={{ paddingLeft: INDENT + 24 }}
            >
              <Loader2 className="h-3 w-3 animate-spin" /> Rendering…
            </div>
          )}
          {tree.map((node) => (
            <FileNode
              key={node.path}
              env={env.id}
              node={node}
              depth={1}
              expanded={expanded}
              onToggle={onToggle}
              active={here ? active : undefined}
              onOpen={onOpen}
            />
          ))}
        </div>
      )}
    </>
  );
}

function FileNode({
  env,
  node,
  depth,
  expanded,
  onToggle,
  active,
  onOpen,
}: {
  env: string;
  node: FileTreeNode;
  depth: number;
  expanded: Set<string>;
  onToggle: (key: string, open?: boolean) => void;
  active: EditorTab | undefined;
  onOpen: (tab: EditorTab) => void;
}) {
  if (node.children) {
    const key = `${env}:${node.path}`;
    const open = expanded.has(key);
    return (
      <>
        <Row depth={depth} disclosure={open} onClick={() => onToggle(key)}>
          {open ? (
            <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          ) : (
            <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          )}
          <span className="truncate">{node.name}</span>
        </Row>
        {open &&
          node.children.map((child) => (
            <FileNode
              key={child.path}
              env={env}
              node={child}
              depth={depth + 1}
              expanded={expanded}
              onToggle={onToggle}
              active={active}
              onOpen={onOpen}
            />
          ))}
      </>
    );
  }
  return (
    <Row
      depth={depth}
      selected={active?.path === node.path}
      onClick={() => onOpen({ env, path: node.path })}
    >
      <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{node.name}</span>
    </Row>
  );
}
