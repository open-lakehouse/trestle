import {
  Button,
  Card,
  cn,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, PanelLeft, PanelLeftClose } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { FilesPanel } from "./artifacts/FilesPanel";
import { MarkitectureCanvas } from "./graph/MarkitectureCanvas";
import { ConfigPanel } from "./layout/ConfigPanel";
import { PlannerProvider, usePlanner } from "./planner";
import type { CatalogDto, Planner, PlanResult, Selection } from "./types";

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

const EMPTY_SELECTION: Selection = {
  modules: [],
  capabilities: [],
  knob_overrides: {},
};

/**
 * The environment editor: pick technologies/capabilities → tune knobs →
 * generate → inspect the topology diagram and rendered files.
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
  const [catalog, setCatalog] = useState<CatalogDto | undefined>(seededCatalog);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [plan, setPlan] = useState<PlanResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [generating, setGenerating] = useState(false);
  const [selectedNodeId, setSelectedNodeId] = useState<string | undefined>();
  const [configCollapsed, setConfigCollapsed] = useState(false);
  const [mobileConfigOpen, setMobileConfigOpen] = useState(false);
  const [workspaceTab, setWorkspaceTab] = useState<"topology" | "files">(
    "topology",
  );

  useEffect(() => {
    if (seededCatalog) {
      setSelection((s) => mergeDefault(s, seededCatalog.default_selection));
      return;
    }
    let cancelled = false;
    planner
      .catalog()
      .then((c) => {
        if (cancelled) return;
        setCatalog(c);
        setSelection((s) => mergeDefault(s, c.default_selection));
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [planner, seededCatalog]);

  const toggleModule = useCallback((moduleId: string) => {
    setSelection((s) => {
      const has = s.modules.includes(moduleId);
      return {
        ...s,
        modules: has
          ? s.modules.filter((m) => m !== moduleId)
          : [...s.modules, moduleId],
      };
    });
  }, []);

  const toggleCapability = useCallback((capability: string) => {
    setSelection((s) => {
      const has = s.capabilities.includes(capability);
      return {
        ...s,
        capabilities: has
          ? s.capabilities.filter((c) => c !== capability)
          : [...s.capabilities, capability],
      };
    });
  }, []);

  const setKnob = useCallback(
    (moduleId: string, key: string, value: string) => {
      setSelection((s) => ({
        ...s,
        knob_overrides: {
          ...s.knob_overrides,
          [moduleId]: { ...s.knob_overrides[moduleId], [key]: value },
        },
      }));
    },
    [],
  );

  const generate = useCallback(async () => {
    setGenerating(true);
    setError(null);
    try {
      const result = await planner.plan(selection);
      setPlan(result);
      setSelectedNodeId(undefined);
      setWorkspaceTab("topology");
      setMobileConfigOpen(false);
    } catch (e) {
      setPlan(null);
      setError(errorMessage(e));
    } finally {
      setGenerating(false);
    }
  }, [planner, selection]);

  const canGenerate =
    selection.modules.length > 0 || selection.capabilities.length > 0;

  const graph = useMemo(() => plan?.graph, [plan]);

  const configPanelProps = {
    catalog,
    selection,
    generating,
    canGenerate,
    onToggleModule: toggleModule,
    onToggleCapability: toggleCapability,
    onSetKnob: setKnob,
    onGenerate: () => void generate(),
  };

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0">
        <aside
          className={cn(
            "hidden min-h-0 shrink-0 flex-col border-r bg-sidebar transition-[width] duration-200 lg:flex",
            configCollapsed ? "w-12" : "w-[22rem]",
          )}
        >
          {configCollapsed ? (
            <div className="flex flex-col items-center gap-2 p-2">
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label="Expand configuration panel"
                onClick={() => setConfigCollapsed(false)}
              >
                <PanelLeft className="h-4 w-4" />
              </Button>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between border-b px-3 py-2">
                <span className="text-sm font-semibold text-sidebar-foreground">
                  Configuration
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  aria-label="Collapse configuration panel"
                  onClick={() => setConfigCollapsed(true)}
                >
                  <PanelLeftClose className="h-4 w-4" />
                </Button>
              </div>
              <div className="min-h-0 flex-1">
                <ConfigPanel {...configPanelProps} />
              </div>
            </>
          )}
        </aside>

        <Sheet open={mobileConfigOpen} onOpenChange={setMobileConfigOpen}>
          <SheetContent
            side="left"
            className="flex w-[min(100vw,22rem)] flex-col p-0"
          >
            <SheetHeader>
              <SheetTitle>Configuration</SheetTitle>
            </SheetHeader>
            <div className="min-h-0 flex-1">
              <ConfigPanel {...configPanelProps} />
            </div>
          </SheetContent>
        </Sheet>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col p-3">
          <Tabs
            value={workspaceTab}
            onValueChange={(value) =>
              setWorkspaceTab(value as "topology" | "files")
            }
            className="flex min-h-0 flex-1 flex-col"
          >
            <div className="mb-3 flex items-center justify-between gap-2">
              <TabsList>
                <TabsTrigger value="topology">Topology</TabsTrigger>
                <TabsTrigger value="files" disabled={!plan}>
                  Files
                </TabsTrigger>
              </TabsList>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="lg:hidden"
                onClick={() => setMobileConfigOpen(true)}
              >
                <PanelLeft className="h-4 w-4" />
                Configure
              </Button>
            </div>

            {error && (
              <Card className="mb-3 flex items-start gap-2 border-destructive/40 p-3 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span className="min-w-0 break-words">{error}</span>
              </Card>
            )}

            <TabsContent
              value="topology"
              className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
            >
              <Card className="h-full min-h-[280px] overflow-hidden p-0">
                {graph ? (
                  <MarkitectureCanvas
                    graph={graph}
                    selectedId={selectedNodeId}
                    onSelect={(n) => setSelectedNodeId(n.id)}
                    colorMode={colorMode}
                  />
                ) : (
                  <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
                    Select technologies or capabilities, then generate to see
                    the environment topology.
                  </div>
                )}
              </Card>
            </TabsContent>

            <TabsContent
              value="files"
              className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
            >
              <Card className="h-full min-h-[280px] overflow-hidden p-0">
                <FilesPanel plan={plan} />
              </Card>
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </TooltipProvider>
  );
}

/** Seed the working selection from the catalog default the first time only. */
function mergeDefault(current: Selection, def: Selection): Selection {
  if (
    current.modules.length > 0 ||
    current.capabilities.length > 0 ||
    Object.keys(current.knob_overrides).length > 0
  ) {
    return current;
  }
  return {
    modules: [...def.modules],
    capabilities: [...def.capabilities],
    knob_overrides: { ...def.knob_overrides },
  };
}

function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}
