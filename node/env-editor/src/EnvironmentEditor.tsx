import {
  Button,
  Card,
  Separator,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import type { ColorMode } from "@xyflow/react";
import { AlertTriangle, Loader2, Play } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ArtifactsPanel } from "./artifacts/ArtifactsPanel";
import { MarkitectureCanvas } from "./graph/MarkitectureCanvas";
import { KnobsStep } from "./knobs/KnobsStep";
import { PlannerProvider, usePlanner } from "./planner";
import { SelectionStep } from "./selection/SelectionStep";
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
 * generate → inspect the "markitecture" diagram and the rendered artifacts.
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

  // Load the catalog once (unless seeded), then open pre-populated with the
  // catalog's default selection.
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

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0 flex-col gap-4 p-4">
        <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
          {/* Left: selection + knobs + generate */}
          <div className="flex min-h-0 flex-col gap-4 overflow-auto">
            {catalog ? (
              <>
                <SelectionStep
                  catalog={catalog}
                  selectedModules={selection.modules}
                  selectedCapabilities={selection.capabilities}
                  onToggleModule={toggleModule}
                  onToggleCapability={toggleCapability}
                />
                <Separator />
                <KnobsStep
                  catalog={catalog}
                  selectedModules={selection.modules}
                  overrides={selection.knob_overrides}
                  onSetKnob={setKnob}
                />
              </>
            ) : (
              <p className="text-sm text-muted-foreground">Loading catalog…</p>
            )}
            <div className="sticky bottom-0 bg-background pt-2">
              <Button
                type="button"
                onClick={() => void generate()}
                disabled={!canGenerate || generating}
                className="w-full"
              >
                {generating ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                Generate environment
              </Button>
            </div>
          </div>

          {/* Right: diagram + artifacts */}
          <div className="flex min-h-0 flex-col gap-4">
            {error && (
              <Card className="flex items-start gap-2 border-destructive/40 p-3 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span className="min-w-0 break-words">{error}</span>
              </Card>
            )}
            <Card className="min-h-[280px] flex-1 overflow-hidden p-0">
              {graph ? (
                <MarkitectureCanvas
                  graph={graph}
                  selectedId={selectedNodeId}
                  onSelect={(n) => setSelectedNodeId(n.id)}
                  colorMode={colorMode}
                />
              ) : (
                <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
                  Select technologies or capabilities, then generate to see the
                  environment topology.
                </div>
              )}
            </Card>
            {plan && <ArtifactsPanel plan={plan} />}
          </div>
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
