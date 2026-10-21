import {
  Button,
  Card,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, Plus } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { buildBundleZip, generateCredentials } from "./artifacts/bundleZip";
import { FilesPanel } from "./artifacts/FilesPanel";
import { fileSourceOf } from "./artifacts/fileSource";
import { downloadBlob } from "./diagram/exportImage";
import { CanvasNotice, CanvasToolbar } from "./graph/CanvasToolbar";
import { ComponentPalette } from "./graph/ComponentPalette";
import type { CanvasActions } from "./graph/canvasActions";
import { MarkitectureCanvas } from "./graph/MarkitectureCanvas";
import type { NodeFilesTarget } from "./graph/nodeFiles";
import { DIRECT, gatewayEnabled, OPEN } from "./inspector/AccessSection";
import { InspectorPanel } from "./inspector/InspectorPanel";
import { ConfigPanel } from "./layout/ConfigPanel";
import { PlannerProvider, usePlanner } from "./planner";
import { moduleName } from "./selection/catalogIndex";
import { componentOrigin, describeOrigin } from "./state/origin";
import { isEmptySelection, useEnvironment } from "./state/useEnvironment";
import type { CatalogDto, Planner, PlanResult } from "./types";

export interface EnvironmentEditorProps {
  /**
   * The planner engine. Optional: if given, the editor wraps itself in a
   * `PlannerProvider`; otherwise it reads one from an ancestor provider. Passing
   * a fixture planner here (or via the provider) is how the component runs in
   * Storybook/tests without wasm.
   */
  planner?: Planner;
  /** Pre-seed the catalog to skip the async `catalog()` fetch (e.g. in stories). */
  catalog?: CatalogDto;
  /** Drives the diagram canvas colors; defaults to `"system"`. */
  colorMode?: ColorMode;
}

type WorkspaceTab = "canvas" | "components" | "files";

/** How long an informational canvas notice stays up. */
const NOTICE_MS = 6000;

/**
 * The environment editor, built around its topology canvas: add components
 * from a palette, configure, swap or remove them from each node's action bar
 * or the inspector, and watch the environment replan live. The Components
 * view lists the same selection as toggles, and Files shows what it renders.
 *
 * Headless: all planning goes through the injected `Planner` seam, so the same
 * component drives a live wasm planner in an app and a fixture planner in tests.
 */
export function EnvironmentEditor({
  planner,
  catalog,
  colorMode,
}: EnvironmentEditorProps) {
  if (planner) {
    return (
      <PlannerProvider planner={planner}>
        <EditorBody catalog={catalog} colorMode={colorMode} />
      </PlannerProvider>
    );
  }
  return <EditorBody catalog={catalog} colorMode={colorMode} />;
}

function EditorBody({
  catalog: seededCatalog,
  colorMode = "system",
}: Pick<EnvironmentEditorProps, "catalog" | "colorMode">) {
  const planner = usePlanner();
  const env = useEnvironment(seededCatalog);
  const { catalog, selection, plan, error, planning, undo, redo } = env;
  const graph = plan?.graph;
  const [tab, setTab] = useState<WorkspaceTab>("canvas");
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [settingsSeq, setSettingsSeq] = useState(0);
  const [resourcesSeq, setResourcesSeq] = useState(0);
  const [focus, setFocus] = useState<{ id: string; seq: number }>();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [filePath, setFilePath] = useState<string | undefined>();
  /** The component whose files are open in the files dialog. */
  const [openFiles, setOpenFiles] = useState<NodeFilesTarget | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** A removal awaiting its replan, to explain a component that stays. */
  const pendingRemoval = useRef<{ id: string; before: PlanResult | null }>(
    null,
  );

  const planFiles = plan?.files;
  const files = useMemo(
    () => (planFiles ? fileSourceOf(planFiles) : undefined),
    [planFiles],
  );

  /** The components in the current plan (everything but the clients). */
  const planned = useMemo(
    () =>
      new Set(
        graph?.nodes.filter((n) => n.kind !== "clients").map((n) => n.id) ?? [],
      ),
    [graph],
  );

  /** A hosted provider is planned: its secrets stay placeholders in a
   *  download. */
  const plansHosted = useMemo(
    () => !!catalog?.modules.some((m) => m.hosted && planned.has(m.id)),
    [catalog, planned],
  );
  const [bundling, setBundling] = useState(false);
  const downloadBundle = useCallback(() => {
    if (!planner.bundle) return;
    setBundling(true);
    const credentials = generateCredentials(catalog?.local_credentials ?? []);
    planner
      .bundle(selection, undefined, credentials)
      .then((bundle) => {
        const zip = buildBundleZip(bundle);
        downloadBlob(
          new Blob([zip as BlobPart], { type: "application/zip" }),
          `${bundle.env_name}.zip`,
        );
        if (plansHosted) {
          setNotice(
            "The hosted resources' secrets aren't in the download: its README.md says how to set them.",
          );
        }
      })
      .catch((e: unknown) => {
        const reason = e instanceof Error ? e.message : String(e);
        setNotice(`Could not prepare the download: ${reason}`);
      })
      .finally(() => setBundling(false));
  }, [planner, catalog, selection, plansHosted]);

  const originOf = useCallback(
    (id: string) => componentOrigin(id, selection, catalog, graph),
    [selection, catalog, graph],
  );

  const hasSettings = useCallback(
    (id: string) =>
      (catalog?.modules.find((m) => m.id === id)?.knobs.length ?? 0) > 0,
    [catalog],
  );

  const alternativesOf = useCallback(
    (id: string) => {
      const role = catalog?.modules.find((m) => m.id === id)?.provider_of;
      if (!catalog || !role) return [];
      return catalog.modules.filter(
        (m) => m.id !== id && m.provider_of === role && !planned.has(m.id),
      );
    },
    [catalog, planned],
  );

  const configure = useCallback((id: string) => {
    setSelectedId(id);
    setSettingsSeq((n) => n + 1);
  }, []);

  const removeComponent = useCallback(
    (id: string) => {
      pendingRemoval.current = { id, before: plan };
      env.remove(id);
      setSelectedId((current) => (current === id ? undefined : current));
    },
    [env.remove, plan],
  );

  const swapComponent = useCallback(
    (fromId: string, toId: string) => {
      env.swap(fromId, toId);
      setSelectedId((current) => (current === fromId ? toId : current));
    },
    [env.swap],
  );

  const addComponent = useCallback(
    (id: string) => {
      env.add(id);
      setSelectedId(id);
    },
    [env.add],
  );

  const manageResources = useCallback((id: string) => {
    setSelectedId(id);
    setResourcesSeq((n) => n + 1);
  }, []);

  /** Select a component and pan the canvas to it. */
  const goTo = useCallback((id: string) => {
    setSelectedId(id);
    setFocus((f) => ({ id, seq: (f?.seq ?? 0) + 1 }));
  }, []);

  const openFile = useCallback((path: string) => {
    setFilePath(path);
    setTab("files");
  }, []);

  const actions = useMemo<CanvasActions>(
    () => ({
      originOf,
      hasSettings,
      alternativesOf,
      configure,
      remove: removeComponent,
      swap: swapComponent,
      addResource: manageResources,
      removeResource: env.removeResource,
    }),
    [
      originOf,
      hasSettings,
      alternativesOf,
      configure,
      removeComponent,
      swapComponent,
      manageResources,
      env.removeResource,
    ],
  );

  // A removed component can stay, pulled in by something else: say why once
  // its replan lands.
  useEffect(() => {
    const pending = pendingRemoval.current;
    if (!pending || !plan || plan === pending.before) return;
    pendingRemoval.current = null;
    if (plan.graph.nodes.some((n) => n.id === pending.id)) {
      const origin = componentOrigin(
        pending.id,
        selection,
        catalog,
        plan.graph,
      );
      setNotice(
        `${moduleName(catalog, pending.id)} stays in the environment. ${describeOrigin(origin)}.`,
      );
    }
  }, [plan, selection, catalog]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), NOTICE_MS);
    return () => clearTimeout(timer);
  }, [notice]);

  // Drop a selection that left the environment (e.g. undoing its add).
  useEffect(() => {
    if (
      selectedId &&
      !planned.has(selectedId) &&
      !selection.modules.includes(selectedId)
    ) {
      setSelectedId(undefined);
    }
  }, [selectedId, planned, selection]);

  // Canvas shortcuts: ⌘K or / opens the component palette, ⌘Z / ⇧⌘Z undo and redo,
  // Delete removes the selected component, Esc deselects.
  useEffect(() => {
    if (tab !== "canvas") return;
    const onKeyDown = (e: KeyboardEvent) => {
      // The files dialog is modal: its own keys (Escape, the tab arrows) only.
      if (openFiles) return;
      const mod = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      if (mod && key === "k") {
        e.preventDefault();
        setPaletteOpen(true);
        return;
      }
      if (paletteOpen || isEditableTarget(e.target)) return;
      if (key === "/") {
        e.preventDefault();
        setPaletteOpen(true);
      } else if (mod && (key === "z" || key === "y")) {
        e.preventDefault();
        if (key === "y" || e.shiftKey) redo();
        else undo();
      } else if (selectedId && key === "escape") {
        setSelectedId(undefined);
      } else if (
        selectedId &&
        (key === "delete" || key === "backspace") &&
        originOf(selectedId).kind === "explicit"
      ) {
        e.preventDefault();
        removeComponent(selectedId);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    tab,
    paletteOpen,
    openFiles,
    selectedId,
    originOf,
    removeComponent,
    undo,
    redo,
  ]);

  const inspected = selectedId && selectedId !== "@clients" ? selectedId : null;
  // On the canvas, plan errors show on the canvas itself.
  const showErrorCard = !!error && (tab !== "canvas" || !catalog);

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0 flex-col p-3">
        <Tabs
          value={tab}
          onValueChange={(value) => setTab(value as WorkspaceTab)}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div className="mb-3 flex items-center gap-3">
            <TabsList>
              <TabsTrigger value="canvas">Canvas</TabsTrigger>
              <TabsTrigger value="components">Components</TabsTrigger>
              <TabsTrigger value="files">Files</TabsTrigger>
            </TabsList>
            {planning && tab !== "canvas" && (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Planning environment…
              </span>
            )}
          </div>

          {showErrorCard && (
            <Card className="mb-3 flex items-start gap-2 border-destructive/40 p-3 text-sm text-destructive">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span className="min-w-0 break-words">{error}</span>
            </Card>
          )}

          <TabsContent
            value="canvas"
            className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
          >
            <Card className="flex h-full min-h-[280px] overflow-hidden p-0">
              <div className="relative min-w-0 flex-1">
                <MarkitectureCanvas
                  graph={graph}
                  selectedId={selectedId}
                  onSelect={(n) => setSelectedId(n.id)}
                  onDeselect={() => setSelectedId(undefined)}
                  actions={actions}
                  focus={focus}
                  colorMode={colorMode}
                  files={files}
                  openFiles={openFiles}
                  onOpenFilesChange={setOpenFiles}
                  onOpenInExplorer={openFile}
                >
                  <CanvasToolbar
                    onAdd={() => setPaletteOpen(true)}
                    onUndo={undo}
                    onRedo={redo}
                    canUndo={env.canUndo}
                    canRedo={env.canRedo}
                    planning={planning}
                    componentCount={planned.size}
                    hasError={!!error}
                    graph={graph}
                    files={files?.paths}
                    deploy={plan?.deploy}
                    onExportError={setNotice}
                    onDownloadBundle={
                      planner.bundle ? downloadBundle : undefined
                    }
                    bundling={bundling}
                    bundleLabel={
                      plansHosted
                        ? "Download environment (.zip) — hosted secrets are placeholders, see its README"
                        : "Download environment (.zip) with fresh credentials"
                    }
                    gatewayEnabled={gatewayEnabled(selection.gateway)}
                    onToggleGateway={(on) => env.setGateway(on ? OPEN : DIRECT)}
                  />
                  <CanvasNotice
                    error={catalog ? error : null}
                    notice={notice}
                    onDismissNotice={() => setNotice(null)}
                  />
                </MarkitectureCanvas>
                {(!catalog || isEmptySelection(selection)) && (
                  <EmptyCanvas
                    loading={!catalog && !error}
                    onAdd={() => setPaletteOpen(true)}
                    onBaseline={catalog ? env.applyDefault : undefined}
                  />
                )}
              </div>
              {inspected && (
                <InspectorPanel
                  moduleId={inspected}
                  catalog={catalog}
                  plan={plan}
                  overrides={selection.knob_overrides}
                  origin={originOf(inspected)}
                  alternatives={alternativesOf(inspected)}
                  settingsSeq={settingsSeq}
                  resourcesSeq={resourcesSeq}
                  onAddResource={(provider, kind, name) =>
                    env.addResource(provider, kind, name)
                  }
                  onRemoveResource={env.removeResource}
                  hosted={selection.hosted?.[inspected]}
                  onSetHosted={env.setHosted}
                  onSetKnob={env.setKnob}
                  gateway={selection.gateway}
                  onSetGateway={env.setGateway}
                  onRemove={removeComponent}
                  onSwap={swapComponent}
                  onSelect={goTo}
                  onOpenFile={(path) => setOpenFiles({ id: inspected, path })}
                  onClose={() => setSelectedId(undefined)}
                />
              )}
            </Card>
            <ComponentPalette
              open={paletteOpen}
              onOpenChange={setPaletteOpen}
              catalog={catalog}
              selection={selection}
              planned={planned}
              originOf={originOf}
              onAdd={addComponent}
              onRemove={removeComponent}
              onSwap={swapComponent}
              onToggleCapability={env.toggleCapability}
            />
          </TabsContent>

          <TabsContent
            value="components"
            className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
          >
            <Card className="h-full min-h-[280px] overflow-hidden p-0">
              <ConfigPanel
                catalog={catalog}
                selection={selection}
                onToggleModule={env.toggleModule}
                onToggleCapability={env.toggleCapability}
                onSetKnob={env.setKnob}
                onSetGateway={env.setGateway}
              />
            </Card>
          </TabsContent>

          <TabsContent
            value="files"
            className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
          >
            <Card className="h-full min-h-[280px] overflow-hidden p-0">
              <FilesPanel plan={plan} initialPath={filePath} />
            </Card>
          </TabsContent>
        </Tabs>
      </div>
    </TooltipProvider>
  );
}

/** The canvas before there's anything on it. */
function EmptyCanvas({
  loading,
  onAdd,
  onBaseline,
}: {
  loading: boolean;
  onAdd: () => void;
  onBaseline?: () => void;
}) {
  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6">
      <div className="pointer-events-auto flex max-w-sm flex-col items-center gap-3 rounded-lg border border-dashed border-border bg-card/95 p-6 text-center shadow-sm">
        {loading ? (
          <span className="inline-flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading catalog…
          </span>
        ) : (
          <>
            <div className="text-sm font-semibold text-card-foreground">
              Start building your environment
            </div>
            <p className="text-xs text-muted-foreground">
              Add components one at a time, or start from the lakehouse baseline
              and adjust it.
            </p>
            <div className="flex gap-2">
              <Button type="button" size="sm" onClick={onAdd}>
                <Plus className="h-4 w-4" />
                Add component
              </Button>
              {onBaseline && (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={onBaseline}
                >
                  Use the baseline
                </Button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Keys typed into a form control or an open menu aren't canvas shortcuts. */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return true;
  return !!target.closest('[role="dialog"], [role="listbox"], [role="menu"]');
}
