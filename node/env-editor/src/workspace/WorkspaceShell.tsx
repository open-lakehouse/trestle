// The workspace editor, laid out like an IDE (Zed, VS Code): an activity bar,
// a sidebar (the workspace's environments, the module catalog, settings), a tab
// per open environment hosting the environment editor, a bottom panel
// (problems across the workspace, how to run the active environment) and a
// status bar. Everything persists through the injected `WorkspaceStore` (see
// `WorkspaceProvider`); all planning goes through the injected `Planner`.

import {
  Button,
  cn,
  Input,
  ResizeHandle,
  Switch,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  usePaneSize,
} from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import {
  AlertTriangle,
  Blocks,
  CircleCheck,
  CircleX,
  Copy,
  Download,
  Files,
  Layers,
  Loader2,
  Network,
  PanelBottom,
  PanelLeft,
  Pencil,
  Plus,
  Settings,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { buildWorkspaceZip, generateCredentials } from "../artifacts/bundleZip";
import { downloadBlob } from "../diagram/exportImage";
import { EnvironmentEditor } from "../EnvironmentEditor";
import { usePlanner } from "../planner";
import type { Bundle, CatalogDto } from "../types";
import {
  contextFor,
  type EnvDoc,
  freeName,
  meshNetwork,
  type PortConflict,
  portConflicts,
  projectName,
  SLOT_PORT_STRIDE,
  type WorkspaceDoc,
  workspaceToml,
} from "./model";
import { gatewayUrl, type MemberPlan, useMemberPlans } from "./useMemberPlans";
import { useWorkspace } from "./useWorkspace";

export interface WorkspaceShellProps {
  /** Drives the diagram canvas colors; defaults to `"system"`. */
  colorMode?: ColorMode;
  /** Pre-seed the catalog to skip the async `catalog()` fetch (e.g. in stories). */
  catalog?: CatalogDto;
  /** Extra status-bar items on the right (e.g. a theme toggle). */
  statusItems?: ReactNode;
}

type SideView = "explorer" | "modules" | "settings";
type PanelView = "problems" | "output";

/** The editor for a whole workspace (see the module comment). */
export function WorkspaceShell({
  colorMode = "system",
  catalog: seededCatalog,
  statusItems,
}: WorkspaceShellProps) {
  const planner = usePlanner();
  const ws = useWorkspace();
  const workspace = ws.workspace;
  const [catalog, setCatalog] = useState<CatalogDto | undefined>(seededCatalog);
  useEffect(() => {
    if (seededCatalog) return;
    let cancelled = false;
    planner.catalog().then((c) => {
      if (!cancelled) setCatalog(c);
    });
    return () => {
      cancelled = true;
    };
  }, [planner, seededCatalog]);

  const plans = useMemberPlans(workspace, catalog);
  const [view, setView] = useState<SideView>("explorer");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [panelOpen, setPanelOpen] = useState(true);
  const [panel, setPanel] = useState<PanelView>("problems");
  const [sidebarWidth, resizeSidebar] = usePaneSize(260, {
    min: 180,
    max: 480,
  });
  const [panelHeight, resizePanel] = usePaneSize(180, {
    min: 96,
    max: 480,
    invert: true,
  });

  // Open tabs, by env id, and the active one. Members that leave the
  // workspace close their tab.
  const [open, setOpen] = useState<string[]>([]);
  const [active, setActive] = useState<string>();
  const envs = workspace?.envs ?? [];
  useEffect(() => {
    if (!workspace) return;
    const ids = new Set(workspace.envs.map((e) => e.id));
    setOpen((tabs) => {
      const kept = tabs.filter((id) => ids.has(id));
      // Start with the first environment open.
      if (kept.length === 0 && workspace.envs[0]) return [workspace.envs[0].id];
      return kept.length === tabs.length ? tabs : kept;
    });
  }, [workspace]);
  useEffect(() => {
    if (!active || !open.includes(active)) setActive(open.at(-1));
  }, [open, active]);
  const openEnv = useCallback((id: string) => {
    setOpen((tabs) => (tabs.includes(id) ? tabs : [...tabs, id]));
    setActive(id);
  }, []);
  const closeEnv = useCallback((id: string) => {
    setOpen((tabs) => tabs.filter((t) => t !== id));
  }, []);
  const activeEnv = envs.find((e) => e.id === active);

  // ⌘B toggles the sidebar, ⌘J the panel, ⌘1…9 switch tabs.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      const key = e.key.toLowerCase();
      if (key === "b") {
        e.preventDefault();
        setSidebarOpen((o) => !o);
      } else if (key === "j") {
        e.preventDefault();
        setPanelOpen((o) => !o);
      } else if (/^[1-9]$/.test(key)) {
        const id = open[Number(key) - 1];
        if (id) {
          e.preventDefault();
          setActive(id);
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open]);

  const conflicts = useMemo(
    () =>
      portConflicts(
        envs.map((env) => ({
          env: env.name,
          hostPorts: plans[env.id]?.plan?.host_ports ?? [],
        })),
      ),
    [envs, plans],
  );
  const failing = envs.filter((e) => plans[e.id]?.error);
  const problemCount = conflicts.length + failing.length;

  const [downloading, setDownloading] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const downloadWorkspace = useCallback(() => {
    if (!workspace || !catalog || !planner.bundle) return;
    const bundle = planner.bundle.bind(planner);
    setDownloading(true);
    Promise.all(
      workspace.envs.map(
        (env): Promise<Bundle> =>
          bundle(
            env.selection ?? catalog.default_selection,
            undefined,
            generateCredentials(catalog.local_credentials ?? []),
            contextFor(workspace, env),
          ),
      ),
    )
      .then((bundles) => {
        const zip = buildWorkspaceZip(
          workspace.name,
          workspaceToml(workspace),
          bundles,
        );
        downloadBlob(
          new Blob([zip as BlobPart], { type: "application/zip" }),
          `${workspace.name}.zip`,
        );
        setNotice(null);
      })
      .catch((e: unknown) =>
        setNotice(
          `Could not prepare the download: ${e instanceof Error ? e.message : String(e)}`,
        ),
      )
      .finally(() => setDownloading(false));
  }, [workspace, catalog, planner]);

  if (!workspace) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        {ws.error ? (
          <span className="text-destructive">{ws.error}</span>
        ) : (
          <span className="inline-flex items-center gap-2">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading workspace…
          </span>
        )}
      </div>
    );
  }

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0 flex-col bg-background text-foreground">
        <div className="flex min-h-0 flex-1">
          <ActivityBar
            view={sidebarOpen ? view : undefined}
            onView={(v) => {
              if (sidebarOpen && v === view) setSidebarOpen(false);
              else {
                setView(v);
                setSidebarOpen(true);
              }
            }}
            problemCount={problemCount}
          />
          {sidebarOpen && (
            <>
              <aside
                className="flex shrink-0 flex-col overflow-hidden bg-sidebar text-sidebar-foreground"
                style={{ width: sidebarWidth }}
              >
                {view === "explorer" && (
                  <ExplorerView
                    workspace={workspace}
                    plans={plans}
                    conflicts={conflicts}
                    active={active}
                    onOpen={openEnv}
                    onDownload={planner.bundle ? downloadWorkspace : undefined}
                    downloading={downloading}
                  />
                )}
                {view === "modules" && <ModulesView catalog={catalog} />}
                {view === "settings" && <SettingsView workspace={workspace} />}
              </aside>
              <ResizeHandle
                orientation="vertical"
                size={sidebarWidth}
                aria-label="Resize sidebar"
                onResize={resizeSidebar}
              />
            </>
          )}
          <main className="flex min-w-0 flex-1 flex-col">
            <TabStrip
              envs={open
                .map((id) => envs.find((e) => e.id === id))
                .filter((e): e is EnvDoc => !!e)}
              active={active}
              plans={plans}
              onSelect={setActive}
              onClose={closeEnv}
            />
            <div className="min-h-0 flex-1">
              {activeEnv ? (
                <EnvironmentEditor
                  key={activeEnv.id}
                  catalog={catalog}
                  colorMode={colorMode}
                  initialSelection={activeEnv.selection}
                  context={contextFor(workspace, activeEnv)}
                  onSelectionChange={(s) => ws.setSelection(activeEnv.id, s)}
                />
              ) : (
                <EmptyEditor
                  onAdd={() => {
                    const env = ws.addEnv(freeName(workspace, "dev"));
                    if (typeof env !== "string") openEnv(env.id);
                  }}
                />
              )}
            </div>
            {panelOpen && (
              <>
                <ResizeHandle
                  orientation="horizontal"
                  size={panelHeight}
                  aria-label="Resize panel"
                  onResize={resizePanel}
                />
                <BottomPanel
                  height={panelHeight}
                  view={panel}
                  onView={setPanel}
                  onClose={() => setPanelOpen(false)}
                  workspace={workspace}
                  env={activeEnv}
                  plans={plans}
                  conflicts={conflicts}
                  notice={notice}
                  onOpen={openEnv}
                />
              </>
            )}
          </main>
        </div>
        <StatusBar
          workspace={workspace}
          env={activeEnv}
          plan={activeEnv ? plans[activeEnv.id] : undefined}
          problemCount={problemCount}
          sidebarOpen={sidebarOpen}
          panelOpen={panelOpen}
          onToggleSidebar={() => setSidebarOpen((o) => !o)}
          onTogglePanel={() => setPanelOpen((o) => !o)}
          onShowProblems={() => {
            setPanel("problems");
            setPanelOpen(true);
          }}
        >
          {statusItems}
        </StatusBar>
      </div>
    </TooltipProvider>
  );
}

// ---------------------------------------------------------------------------
// Activity bar
// ---------------------------------------------------------------------------

function ActivityBar({
  view,
  onView,
  problemCount,
}: {
  view: SideView | undefined;
  onView: (view: SideView) => void;
  problemCount: number;
}) {
  const items: { id: SideView; label: string; icon: ReactNode }[] = [
    { id: "explorer", label: "Environments", icon: <Files /> },
    { id: "modules", label: "Modules", icon: <Blocks /> },
    { id: "settings", label: "Workspace settings", icon: <Settings /> },
  ];
  return (
    <nav
      aria-label="Views"
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-r bg-activitybar py-2"
    >
      {items.map((item) => (
        <Tooltip key={item.id}>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label={item.label}
              aria-pressed={view === item.id}
              onClick={() => onView(item.id)}
              className={cn(
                "relative flex h-10 w-10 items-center justify-center rounded-md text-muted-foreground transition-colors hover:text-foreground [&_svg]:h-5 [&_svg]:w-5",
                view === item.id &&
                  "text-foreground before:absolute before:inset-y-2 before:-left-1 before:w-0.5 before:rounded-full before:bg-primary",
              )}
            >
              {item.icon}
              {item.id === "explorer" && problemCount > 0 && (
                <span className="absolute right-1 top-1 h-2 w-2 rounded-full bg-destructive" />
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent side="right">{item.label}</TooltipContent>
        </Tooltip>
      ))}
    </nav>
  );
}

// ---------------------------------------------------------------------------
// Sidebar views
// ---------------------------------------------------------------------------

function SidebarHeader({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex h-9 shrink-0 items-center justify-between px-3">
      <span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </span>
      <div className="flex items-center gap-0.5">{children}</div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label={label}
          disabled={disabled}
          onClick={onClick}
          className="h-6 w-6 [&_svg]:h-3.5 [&_svg]:w-3.5"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/** What the explorer is asking a name for. */
type NameEdit =
  | { kind: "add" }
  | { kind: "rename"; id: string }
  | { kind: "duplicate"; id: string };

function ExplorerView({
  workspace,
  plans,
  conflicts,
  active,
  onOpen,
  onDownload,
  downloading,
}: {
  workspace: WorkspaceDoc;
  plans: Record<string, MemberPlan>;
  conflicts: PortConflict[];
  active: string | undefined;
  onOpen: (id: string) => void;
  onDownload?: () => void;
  downloading: boolean;
}) {
  const ws = useWorkspace();
  const [edit, setEdit] = useState<NameEdit | null>(null);
  const clashing = new Set(
    conflicts.flatMap((c) => [c.first.env, c.second.env]),
  );

  const submit = (name: string): string | undefined => {
    if (!edit) return undefined;
    if (edit.kind === "rename") return ws.renameEnv(edit.id, name);
    const env =
      edit.kind === "add" ? ws.addEnv(name) : ws.duplicateEnv(edit.id, name);
    if (typeof env === "string") return env;
    onOpen(env.id);
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
      <ul className="min-h-0 flex-1 overflow-y-auto pb-2">
        {workspace.envs.map((env) =>
          edit && edit.kind === "rename" && edit.id === env.id ? (
            <li key={env.id} className="px-2 py-0.5">
              <NameInput
                initial={env.name}
                onSubmit={submit}
                onDone={() => setEdit(null)}
              />
            </li>
          ) : (
            <EnvRow
              key={env.id}
              env={env}
              plan={plans[env.id]}
              clashing={clashing.has(env.name)}
              active={env.id === active}
              onOpen={() => onOpen(env.id)}
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
          <li className="px-2 py-0.5">
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
          </li>
        )}
      </ul>
    </div>
  );
}

function EnvRow({
  env,
  plan,
  clashing,
  active,
  onOpen,
  onRename,
  onDuplicate,
  onRemove,
}: {
  env: EnvDoc;
  plan: MemberPlan | undefined;
  clashing: boolean;
  active: boolean;
  onOpen: () => void;
  onRename: () => void;
  onDuplicate: () => void;
  onRemove: () => void;
}) {
  const port = plan?.plan?.gateway.listeners[0]?.host_port;
  return (
    <li
      className={cn(
        "group flex h-7 items-center gap-1.5 px-3 text-sm",
        active ? "bg-sidebar-accent" : "hover:bg-sidebar-accent/60",
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
      >
        <EnvStatus plan={plan} clashing={clashing} />
        <span className="truncate">{env.name}</span>
        <span className="ml-auto shrink-0 font-mono text-[11px] text-muted-foreground">
          {port ? `:${port}` : `+${env.slot * SLOT_PORT_STRIDE}`}
        </span>
      </button>
      <div className="hidden shrink-0 items-center group-hover:flex">
        <IconButton label="Rename" onClick={onRename}>
          <Pencil />
        </IconButton>
        <IconButton label="Duplicate (e.g. dev → prod)" onClick={onDuplicate}>
          <Copy />
        </IconButton>
        <IconButton label="Remove" onClick={onRemove}>
          <Trash2 />
        </IconButton>
      </div>
    </li>
  );
}

function EnvStatus({
  plan,
  clashing,
}: {
  plan: MemberPlan | undefined;
  clashing?: boolean;
}) {
  if (!plan) {
    return (
      <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
    );
  }
  if (plan.error) {
    return <CircleX className="h-3.5 w-3.5 shrink-0 text-destructive" />;
  }
  if (clashing) {
    return <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-status-ready" />;
  }
  return <CircleCheck className="h-3.5 w-3.5 shrink-0 text-success" />;
}

/** An inline name field: Enter submits (keeping it open on a refusal), Escape
 *  or leaving cancels. */
function NameInput({
  initial,
  onSubmit,
  onDone,
}: {
  initial: string;
  onSubmit: (name: string) => string | undefined;
  onDone: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [problem, setProblem] = useState<string>();
  return (
    <div>
      <Input
        autoFocus
        value={value}
        aria-label="Environment name"
        aria-invalid={!!problem}
        className="h-7 px-2 text-sm"
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => {
          setValue(e.target.value);
          setProblem(undefined);
        }}
        onBlur={onDone}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
          if (e.key !== "Enter") return;
          const refused = onSubmit(value.trim());
          if (refused) setProblem(refused);
          else onDone();
        }}
      />
      {problem && <p className="mt-1 text-xs text-destructive">{problem}</p>}
    </div>
  );
}

function ModulesView({ catalog }: { catalog: CatalogDto | undefined }) {
  const groups = useMemo(() => {
    const byCategory = new Map<string, CatalogDto["modules"]>();
    for (const module of catalog?.modules ?? []) {
      const category = module.category ?? "other";
      byCategory.set(category, [...(byCategory.get(category) ?? []), module]);
    }
    return [...byCategory.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [catalog]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SidebarHeader title="Modules" />
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3 text-sm">
        {!catalog && (
          <span className="text-xs text-muted-foreground">
            Loading catalog…
          </span>
        )}
        {groups.map(([category, modules]) => (
          <section key={category} className="mb-3">
            <h3 className="mb-1 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              {category}
            </h3>
            <ul>
              {modules.map((m) => (
                <li
                  key={m.id}
                  className="py-0.5"
                  title={m.summary ?? undefined}
                >
                  <span className="text-foreground">
                    {m.display_name ?? m.id}
                  </span>{" "}
                  <span className="font-mono text-[11px] text-muted-foreground">
                    {m.id}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}

function SettingsView({ workspace }: { workspace: WorkspaceDoc }) {
  const ws = useWorkspace();
  const [renaming, setRenaming] = useState(false);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SidebarHeader title="Workspace" />
      <div className="space-y-4 px-3 pb-3 text-sm">
        <div>
          <div className="mb-1 text-xs text-muted-foreground">Name</div>
          {renaming ? (
            <NameInput
              initial={workspace.name}
              onSubmit={(name) => ws.renameWorkspace(name)}
              onDone={() => setRenaming(false)}
            />
          ) : (
            <button
              type="button"
              className="inline-flex items-center gap-1.5 hover:underline"
              onClick={() => setRenaming(true)}
            >
              {workspace.name}
              <Pencil className="h-3 w-3 text-muted-foreground" />
            </button>
          )}
          <p className="mt-1 text-xs text-muted-foreground">
            Prefixes each environment's compose project (
            <code>{workspace.name}-dev</code>).
          </p>
        </div>
        <div className="flex items-start justify-between gap-3">
          <span>
            <span className="block">Gateway mesh</span>
            <span className="block text-xs text-muted-foreground">
              The environments' gateways, and nothing else, share the network{" "}
              <code>trestle-{workspace.name}-mesh</code>, so they reach each
              other at <code>&lt;env&gt;.mesh</code>.
            </span>
          </span>
          <Switch
            checked={workspace.mesh}
            onCheckedChange={ws.setMesh}
            aria-label="Gateway mesh"
          />
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Editor tabs
// ---------------------------------------------------------------------------

function TabStrip({
  envs,
  active,
  plans,
  onSelect,
  onClose,
}: {
  envs: EnvDoc[];
  active: string | undefined;
  plans: Record<string, MemberPlan>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Open environments"
      className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b bg-sidebar"
    >
      {envs.map((env) => (
        <div
          key={env.id}
          className={cn(
            "group flex items-center gap-1.5 border-r pl-3 pr-1 text-sm",
            env.id === active
              ? "bg-background text-foreground shadow-[inset_0_2px_0_hsl(var(--primary))]"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          <button
            type="button"
            role="tab"
            aria-selected={env.id === active}
            onClick={() => onSelect(env.id)}
            onAuxClick={(e) => {
              if (e.button === 1) onClose(env.id);
            }}
            className="flex items-center gap-1.5"
          >
            <EnvStatus plan={plans[env.id]} />
            {env.name}
          </button>
          <button
            type="button"
            aria-label={`Close ${env.name}`}
            onClick={() => onClose(env.id)}
            className={cn(
              "rounded p-0.5 hover:bg-muted",
              env.id === active ? "visible" : "invisible group-hover:visible",
            )}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}

function EmptyEditor({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
      <span>No environment open. Pick one in the explorer, or</span>
      <Button type="button" size="sm" onClick={onAdd}>
        <Plus className="h-4 w-4" />
        New environment
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Bottom panel
// ---------------------------------------------------------------------------

function BottomPanel({
  height,
  view,
  onView,
  onClose,
  workspace,
  env,
  plans,
  conflicts,
  notice,
  onOpen,
}: {
  height: number;
  view: PanelView;
  onView: (view: PanelView) => void;
  onClose: () => void;
  workspace: WorkspaceDoc;
  env: EnvDoc | undefined;
  plans: Record<string, MemberPlan>;
  conflicts: PortConflict[];
  notice: string | null;
  onOpen: (id: string) => void;
}) {
  const failing = workspace.envs.filter((e) => plans[e.id]?.error);
  const count = failing.length + conflicts.length + (notice ? 1 : 0);
  const tabs: { id: PanelView; label: string; icon: ReactNode }[] = [
    {
      id: "problems",
      label: count > 0 ? `Problems (${count})` : "Problems",
      icon: <AlertTriangle />,
    },
    { id: "output", label: "Run", icon: <Terminal /> },
  ];
  return (
    <section
      className="flex shrink-0 flex-col bg-background"
      style={{ height }}
      aria-label="Panel"
    >
      <div className="flex h-8 shrink-0 items-center gap-1 px-2">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => onView(t.id)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium uppercase tracking-wider [&_svg]:h-3.5 [&_svg]:w-3.5",
              view === t.id
                ? "text-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {t.icon}
            {t.label}
          </button>
        ))}
        <div className="ml-auto">
          <IconButton label="Close panel (⌘J)" onClick={onClose}>
            <X />
          </IconButton>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2 font-mono text-xs">
        {view === "problems" ? (
          count === 0 ? (
            <p className="text-muted-foreground">
              No problems: every environment plans, and no two bind the same
              host port.
            </p>
          ) : (
            <ul className="space-y-1">
              {notice && (
                <li className="flex gap-2 text-destructive">
                  <CircleX className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  {notice}
                </li>
              )}
              {failing.map((e) => (
                <li key={e.id} className="flex gap-2">
                  <CircleX className="mt-0.5 h-3.5 w-3.5 shrink-0 text-destructive" />
                  <button
                    type="button"
                    className="text-left hover:underline"
                    onClick={() => onOpen(e.id)}
                  >
                    <span className="font-semibold">{e.name}</span>:{" "}
                    {plans[e.id]?.error}
                  </button>
                </li>
              ))}
              {conflicts.map((c) => (
                <li key={`${c.port}-${c.second.env}`} className="flex gap-2">
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-status-ready" />
                  <span>
                    host port {c.port} is bound by <code>{c.first.owner}</code>{" "}
                    in <b>{c.first.env}</b> and by <code>{c.second.owner}</code>{" "}
                    in <b>{c.second.env}</b>
                  </span>
                </li>
              ))}
            </ul>
          )
        ) : (
          <RunOutput
            workspace={workspace}
            env={env}
            plan={env && plans[env.id]}
          />
        )}
      </div>
    </section>
  );
}

function RunOutput({
  workspace,
  env,
  plan,
}: {
  workspace: WorkspaceDoc;
  env: EnvDoc | undefined;
  plan: MemberPlan | undefined;
}) {
  if (!env) {
    return <p className="text-muted-foreground">No environment open.</p>;
  }
  const url = gatewayUrl(plan?.plan, env.name);
  const mesh = meshNetwork(workspace);
  const lines = [
    "# Download the workspace (explorer ⤓), unzip it, then:",
    `cd ${workspace.name}`,
    `trestle workspace up ${env.name}    # or: cd ${env.name} && docker compose up -d --wait`,
    "",
    `# project   ${projectName(workspace, env)}  (slot ${env.slot}, ports +${env.slot * SLOT_PORT_STRIDE})`,
    `# gateway   ${url ?? "none: each service on its own host port"}`,
  ];
  if (mesh && url) {
    lines.push(`# mesh      ${env.name}.mesh on ${mesh}`);
  }
  lines.push(`trestle workspace down ${env.name}`);
  return <pre className="whitespace-pre-wrap">{lines.join("\n")}</pre>;
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

function StatusBar({
  workspace,
  env,
  plan,
  problemCount,
  sidebarOpen,
  panelOpen,
  onToggleSidebar,
  onTogglePanel,
  onShowProblems,
  children,
}: {
  workspace: WorkspaceDoc;
  env: EnvDoc | undefined;
  plan: MemberPlan | undefined;
  problemCount: number;
  sidebarOpen: boolean;
  panelOpen: boolean;
  onToggleSidebar: () => void;
  onTogglePanel: () => void;
  onShowProblems: () => void;
  children?: ReactNode;
}) {
  const url = env ? gatewayUrl(plan?.plan, env.name) : undefined;
  const item =
    "inline-flex h-full items-center gap-1 px-2 hover:bg-foreground/10 [&_svg]:h-3.5 [&_svg]:w-3.5";
  return (
    <footer className="flex h-6 shrink-0 items-center border-t bg-statusbar text-[11px] text-statusbar-foreground">
      <button
        type="button"
        className={item}
        onClick={onToggleSidebar}
        aria-label="Toggle sidebar (⌘B)"
        aria-pressed={sidebarOpen}
      >
        <PanelLeft />
      </button>
      <span className={item}>
        <Layers />
        {workspace.name}
      </span>
      {env && (
        <span className={item}>
          {env.name} · slot {env.slot}
        </span>
      )}
      <button type="button" className={item} onClick={onShowProblems}>
        {problemCount > 0 ? <AlertTriangle /> : <CircleCheck />}
        {problemCount}
      </button>
      <div className="ml-auto flex h-full items-center">
        {env && (
          <span className={cn(item, "font-mono")}>
            {plan === undefined ? (
              <Loader2 className="animate-spin" />
            ) : (
              (url ?? "no gateway")
            )}
          </span>
        )}
        <button
          type="button"
          className={item}
          onClick={onTogglePanel}
          aria-label="Toggle panel (⌘J)"
          aria-pressed={panelOpen}
        >
          <PanelBottom />
        </button>
        {children}
      </div>
    </footer>
  );
}
