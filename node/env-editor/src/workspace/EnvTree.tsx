// The explorer: the workspace's environments, one row each; picking one opens
// its environment tab (its diagram, files and changes are that tab's
// sub-tabs). Environments are added, renamed, duplicated and removed here,
// from each row's ⋯ menu, beside the host's run action; resting on a row a
// little shows its card.

import { cn, DropdownMenuItem } from "@open-lakehouse/ui-kit";
import {
  Copy,
  Download,
  Layers,
  Loader2,
  Network,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import { type HTMLAttributes, type ReactNode, type Ref, useState } from "react";
import { RunStateDot } from "../graph/RunStateDot.js";
import { runStateTitle } from "../graph/runState.js";
import {
  EnvStatus,
  IconButton,
  InlineConfirm,
  NameInput,
  SidebarHeader,
} from "./chrome.js";
import { type EnvironmentStatus, useContributions } from "./contributions.js";
import type { EditorTab } from "./editors.js";
import { ActionsMenu, RunActionButton, useEnvActions } from "./envActions.js";
import { EnvHoverCard } from "./envCard.js";
import {
  type EnvDoc,
  freeName,
  type PortConflict,
  SLOT_PORT_STRIDE,
  type WorkspaceDoc,
} from "./model.js";
import type { MemberPlan } from "./useMemberPlans.js";
import { useWorkspace } from "./useWorkspace.js";

/** What the explorer is asking a name for. */
type NameEdit =
  | { kind: "add" }
  | { kind: "rename"; id: string }
  | { kind: "duplicate"; id: string };

export interface EnvTreeProps {
  workspace: WorkspaceDoc;
  plans: Record<string, MemberPlan>;
  conflicts: PortConflict[];
  /** The active editor; its environment is highlighted. */
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
  const statusOf = useContributions().contributions?.environment?.status;
  const [edit, setEdit] = useState<NameEdit | null>(null);
  // The environment whose removal is being confirmed.
  const [removing, setRemoving] = useState<string | null>(null);
  const clashing = new Set(
    conflicts.flatMap((c) => [c.first.env, c.second.env]),
  );

  const submit = (name: string): string | undefined => {
    if (!edit) return undefined;
    if (edit.kind === "rename") return ws.renameEnv(edit.id, name);
    const env =
      edit.kind === "add" ? ws.addEnv(name) : ws.duplicateEnv(edit.id, name);
    if (typeof env === "string") return env;
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
              status={statusOf?.(env.id)}
              clashing={clashing.has(env.name)}
              active={active?.env === env.id}
              onOpen={() => onOpen({ env: env.id })}
              onRename={() => setEdit({ kind: "rename", id: env.id })}
              onDuplicate={() => setEdit({ kind: "duplicate", id: env.id })}
              onRemove={() => setRemoving(env.id)}
              confirm={
                removing === env.id && (
                  <div className="px-2 py-0.5">
                    <InlineConfirm
                      message={
                        <>
                          Remove <code>{env.name}</code> from the workspace? Its
                          selection is lost.
                        </>
                      }
                      action="Remove"
                      onConfirm={() => {
                        setRemoving(null);
                        ws.removeEnv(env.id);
                      }}
                      onCancel={() => setRemoving(null)}
                    />
                  </div>
                )
              }
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

/** One environment's row, with its actions on hover (and while its menu is
 *  open). The rest of its props (a hover card trigger's) go to the row. */
function Row({
  selected,
  onClick,
  children,
  actions,
  pinned,
  ref,
  className,
  ...rest
}: {
  selected?: boolean;
  onClick: () => void;
  children: ReactNode;
  actions?: ReactNode;
  /** Keeps the actions shown, e.g. while the row's menu is open. */
  pinned?: boolean;
  ref?: Ref<HTMLDivElement>;
} & Omit<HTMLAttributes<HTMLDivElement>, "onClick">) {
  return (
    <div
      {...rest}
      ref={ref}
      role="treeitem"
      aria-selected={!!selected}
      tabIndex={-1}
      data-pinned={pinned || undefined}
      className={cn(
        "group flex h-6 items-center pl-3 pr-1 text-[13px]",
        selected
          ? "bg-sidebar-accent"
          : pinned
            ? "bg-sidebar-accent/60"
            : "hover:bg-sidebar-accent/60",
        className,
      )}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
      >
        {children}
      </button>
      {actions && (
        <div
          className={cn(
            "shrink-0 items-center",
            pinned ? "flex" : "hidden group-hover:flex",
          )}
        >
          {actions}
        </div>
      )}
    </div>
  );
}

/** The row menu's own items: as compact as the host's. */
const ITEM = "text-xs [&_svg]:size-3.5";

function EnvNode({
  env,
  plan,
  status,
  clashing,
  active,
  onOpen,
  onRename,
  onDuplicate,
  onRemove,
  confirm,
}: {
  env: EnvDoc;
  plan: MemberPlan | undefined;
  /** What the host reports about it, if anything. */
  status?: EnvironmentStatus;
  clashing: boolean;
  /** Its tab is the active editor. */
  active: boolean;
  onOpen: () => void;
  onRename: () => void;
  onDuplicate: () => void;
  onRemove: () => void;
  /** The removal confirmation, drawn under the row while it asks. */
  confirm?: ReactNode;
}) {
  const { run, menu } = useEnvActions(env.id);
  const [menuOpen, setMenuOpen] = useState(false);
  const port = plan?.plan?.gateway.listeners[0]?.host_port;
  const ports = `slot ${env.slot}: host ports +${env.slot * SLOT_PORT_STRIDE}`;
  return (
    <>
      <EnvHoverCard
        envId={env.id}
        name={env.name}
        planStatus={<EnvStatus plan={plan} clashing={clashing} />}
        footnote={ports}
        openDelay={900}
        side="right"
        disabled={menuOpen}
      >
        <Row
          selected={active}
          onClick={onOpen}
          pinned={menuOpen}
          actions={
            <>
              {run && <RunActionButton action={run} />}
              <ActionsMenu
                label={`More actions for ${env.name}`}
                onOpenChange={setMenuOpen}
                leading={
                  <>
                    <DropdownMenuItem onSelect={onRename} className={ITEM}>
                      <Pencil /> Rename
                    </DropdownMenuItem>
                    <DropdownMenuItem onSelect={onDuplicate} className={ITEM}>
                      <Copy /> Duplicate
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onSelect={onRemove}
                      variant="destructive"
                      className={ITEM}
                    >
                      <Trash2 /> Remove
                    </DropdownMenuItem>
                  </>
                }
                actions={menu}
              />
            </>
          }
        >
          <EnvStatus plan={plan} clashing={clashing} />
          <span className={cn("truncate", active && "font-medium")}>
            {env.name}
          </span>
          {status && <RunStateDot status={status} />}
          <span
            className={cn(
              "ml-auto shrink-0 truncate pr-1 font-mono text-[11px] text-muted-foreground",
              menuOpen ? "hidden" : "group-hover:hidden",
            )}
            title={status ? runStateTitle(status) : undefined}
          >
            {status?.placement ??
              (port ? `:${port}` : `+${env.slot * SLOT_PORT_STRIDE}`)}
          </span>
        </Row>
      </EnvHoverCard>
      {confirm}
    </>
  );
}
