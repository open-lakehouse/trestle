import {
  Card,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

type WorkspaceTab = "configure" | "topology" | "files";

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
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>("configure");
  /** Snapshot of the selection that produced the current `plan` (for dirty checks). */
  const plannedSelectionRef = useRef<string | null>(null);

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

  const canGenerate =
    selection.modules.length > 0 || selection.capabilities.length > 0;

  const selectionKey = useMemo(
    () => serializeSelection(selection),
    [selection],
  );
  const isDirty =
    plannedSelectionRef.current === null ||
    plannedSelectionRef.current !== selectionKey;

  const generate = useCallback(async (): Promise<boolean> => {
    if (!canGenerate) {
      setError(
        "Select at least one technology or capability before generating.",
      );
      return false;
    }
    setGenerating(true);
    setError(null);
    try {
      const result = await planner.plan(selection);
      setPlan(result);
      setSelectedNodeId(undefined);
      plannedSelectionRef.current = serializeSelection(selection);
      return true;
    } catch (e) {
      setPlan(null);
      plannedSelectionRef.current = null;
      setError(errorMessage(e));
      return false;
    } finally {
      setGenerating(false);
    }
  }, [planner, selection, canGenerate]);

  const switchTab = useCallback(
    async (next: WorkspaceTab) => {
      if (next === workspaceTab || generating) return;

      // Leaving Configure regenerates when the selection changed (or never planned).
      if (workspaceTab === "configure" && next !== "configure" && isDirty) {
        const ok = await generate();
        if (!ok) return;
      }

      setWorkspaceTab(next);
    },
    [workspaceTab, generating, isDirty, generate],
  );

  const graph = useMemo(() => plan?.graph, [plan]);

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0 flex-col p-3">
        <Tabs
          value={workspaceTab}
          onValueChange={(value) => void switchTab(value as WorkspaceTab)}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div className="mb-3 flex items-center gap-3">
            <TabsList>
              <TabsTrigger value="configure" disabled={generating}>
                Configure
              </TabsTrigger>
              <TabsTrigger value="topology" disabled={generating}>
                Topology
              </TabsTrigger>
              <TabsTrigger value="files" disabled={generating}>
                Files
              </TabsTrigger>
            </TabsList>
            {generating && (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Generating environment…
              </span>
            )}
          </div>

          {error && (
            <Card className="mb-3 flex items-start gap-2 border-destructive/40 p-3 text-sm text-destructive">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span className="min-w-0 break-words">{error}</span>
            </Card>
          )}

          <TabsContent
            value="configure"
            className="mt-0 min-h-0 flex-1 data-[state=inactive]:hidden"
          >
            <Card className="h-full min-h-[280px] overflow-hidden p-0">
              <ConfigPanel
                catalog={catalog}
                selection={selection}
                onToggleModule={toggleModule}
                onToggleCapability={toggleCapability}
                onSetKnob={setKnob}
              />
            </Card>
          </TabsContent>

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
                  Choose technologies on the Configure tab, then switch here to
                  generate and view the environment topology.
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
    </TooltipProvider>
  );
}

/** Stable JSON key for dirty-checking the working selection against the last plan. */
function serializeSelection(selection: Selection): string {
  return JSON.stringify({
    modules: [...selection.modules].sort(),
    capabilities: [...selection.capabilities].sort(),
    knob_overrides: selection.knob_overrides,
  });
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
