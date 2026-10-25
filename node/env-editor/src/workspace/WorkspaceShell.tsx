// The workspace editor, laid out like an IDE (Zed, VS Code): an activity bar,
// a sidebar (the workspace's environments as a tree of diagrams and rendered
// files, the module catalog, settings), editor tabs (an environment's diagram
// hosting the environment editor, or one of its files), a bottom panel
// (problems across the workspace, how to run the active environment) and a
// status bar. Everything persists through the injected `WorkspaceStore` (see
// `WorkspaceProvider`); all planning goes through the injected `Planner`.

import {
  Button,
  cn,
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
  Files,
  FileText,
  Layers,
  Loader2,
  PanelBottom,
  PanelLeft,
  Pencil,
  Plus,
  Settings,
  Terminal,
  X,
} from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { buildWorkspaceZip, generateCredentials } from "../artifacts/bundleZip";
import { CodeViewer } from "../artifacts/CodeViewer";
import { fileSourceOf } from "../artifacts/fileSource";
import { downloadBlob } from "../diagram/exportImage";
import { EnvironmentEditor } from "../EnvironmentEditor";
import { usePlanner } from "../planner";
import type { Bundle, CatalogDto } from "../types";
import {
  Breadcrumb,
  EnvStatus,
  IconButton,
  NameInput,
  SidebarHeader,
} from "./chrome";
import { EnvTree } from "./EnvTree";
import {
  closeTab,
  type EditorTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors";
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

  // Open editor tabs (an environment's diagram, or one of its files) and the
  // active one. A member that leaves the workspace closes its tabs.
  const [open, setOpen] = useState<EditorTab[]>([]);
  const [activeKey, setActiveKey] = useState<string>();
  const envs = workspace?.envs ?? [];
  const started = useRef(false);
  useEffect(() => {
    if (!workspace) return;
    const ids = new Set(workspace.envs.map((e) => e.id));
    setOpen((tabs) => {
      // Start with the first environment's diagram open.
      if (!started.current && workspace.envs[0]) {
        started.current = true;
        return [{ env: workspace.envs[0].id }];
      }
      return pruneTabs(tabs, ids);
    });
  }, [workspace]);
  useEffect(() => {
    if (!activeKey || !open.some((t) => tabKey(t) === activeKey)) {
      const last = open.at(-1);
      setActiveKey(last && tabKey(last));
    }
  }, [open, activeKey]);
  const openEditor = useCallback((tab: EditorTab) => {
    setOpen((tabs) => openTab(tabs, tab));
    setActiveKey(tabKey(tab));
  }, []);
  const openEnv = useCallback(
    (id: string) => openEditor({ env: id }),
    [openEditor],
  );
  const closeEditor = useCallback(
    (key: string) => {
      setOpen((tabs) => {
        if (key === activeKey) setActiveKey(neighbourAfterClose(tabs, key));
        return closeTab(tabs, key);
      });
    },
    [activeKey],
  );
  const activeTab = open.find((t) => tabKey(t) === activeKey);
  const activeEnv = envs.find((e) => e.id === activeTab?.env);

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
        const tab = open[Number(key) - 1];
        if (tab) {
          e.preventDefault();
          setActiveKey(tabKey(tab));
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
                  <EnvTree
                    workspace={workspace}
                    plans={plans}
                    conflicts={conflicts}
                    active={activeTab}
                    onOpen={openEditor}
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
              tabs={open}
              envs={envs}
              activeKey={activeKey}
              plans={plans}
              onSelect={setActiveKey}
              onClose={closeEditor}
            />
            <div className="min-h-0 flex-1">
              {activeEnv && activeTab?.path !== undefined ? (
                <FileEditor
                  workspace={workspace}
                  env={activeEnv}
                  path={activeTab.path}
                  plan={plans[activeEnv.id]}
                />
              ) : activeEnv ? (
                <EnvironmentEditor
                  key={activeEnv.id}
                  chrome="workspace"
                  catalog={catalog}
                  colorMode={colorMode}
                  initialSelection={activeEnv.selection}
                  context={contextFor(workspace, activeEnv)}
                  onSelectionChange={(s) => ws.setSelection(activeEnv.id, s)}
                  onOpenFile={(path) => openEditor({ env: activeEnv.id, path })}
                  toolbarLeading={
                    <Breadcrumb
                      parts={[workspace.name, activeEnv.name, "Diagram"]}
                    />
                  }
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
  tabs,
  envs,
  activeKey,
  plans,
  onSelect,
  onClose,
}: {
  tabs: EditorTab[];
  envs: EnvDoc[];
  activeKey: string | undefined;
  plans: Record<string, MemberPlan>;
  onSelect: (key: string) => void;
  onClose: (key: string) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Open editors"
      className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b bg-sidebar"
    >
      {tabs.map((tab) => {
        const key = tabKey(tab);
        const env = envs.find((e) => e.id === tab.env);
        if (!env) return null;
        const active = key === activeKey;
        const label = tab.path
          ? (tab.path.split("/").at(-1) ?? tab.path)
          : env.name;
        return (
          <div
            key={key}
            title={tab.path ? `${env.name}/${tab.path}` : `${env.name} diagram`}
            className={cn(
              "group flex items-center gap-1.5 border-r pl-3 pr-1 text-sm",
              active
                ? "bg-background text-foreground shadow-[inset_0_2px_0_hsl(var(--primary))]"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <button
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => onSelect(key)}
              onAuxClick={(e) => {
                if (e.button === 1) onClose(key);
              }}
              className="flex items-center gap-1.5 whitespace-nowrap"
            >
              {tab.path ? (
                <FileText className="h-3.5 w-3.5 text-muted-foreground" />
              ) : (
                <EnvStatus plan={plans[env.id]} />
              )}
              {label}
              {tab.path && (
                <span className="text-xs text-muted-foreground">
                  {env.name}
                </span>
              )}
            </button>
            <button
              type="button"
              aria-label={`Close ${label}`}
              onClick={() => onClose(key)}
              className={cn(
                "rounded p-0.5 hover:bg-muted",
                active ? "visible" : "invisible group-hover:visible",
              )}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}

/** A rendered file of `env`, read-only, under a breadcrumb toolbar. */
function FileEditor({
  workspace,
  env,
  path,
  plan,
}: {
  workspace: WorkspaceDoc;
  env: EnvDoc;
  path: string;
  plan: MemberPlan | undefined;
}) {
  const files = plan?.plan?.files;
  const source = useMemo(
    () => (files ? fileSourceOf(files) : undefined),
    [files],
  );
  const exists = !!files?.some((f) => f.path === path);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center border-b px-3">
        <Breadcrumb parts={[workspace.name, env.name, ...path.split("/")]} />
      </div>
      <div className="flex min-h-0 flex-1 flex-col">
        {!plan ? (
          <Centered>
            <Loader2 className="h-4 w-4 animate-spin" /> Rendering…
          </Centered>
        ) : !source || !exists ? (
          <Centered>
            {plan.error
              ? `${env.name} doesn't plan: ${plan.error}`
              : `${env.name} no longer renders ${path}.`}
          </Centered>
        ) : (
          <CodeViewer
            key={path}
            path={path}
            read={source.read}
            header="compact"
          />
        )}
      </div>
    </div>
  );
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full items-center justify-center gap-2 p-6 text-sm text-muted-foreground">
      {children}
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
