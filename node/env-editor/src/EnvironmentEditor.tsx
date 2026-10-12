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
import { useState } from "react";
import { FilesPanel } from "./artifacts/FilesPanel";
import { MarkitectureCanvas } from "./graph/MarkitectureCanvas";
import { ConfigPanel } from "./layout/ConfigPanel";
import { PlannerProvider } from "./planner";
import { useEnvironment } from "./state/useEnvironment";
import type { CatalogDto, Planner } from "./types";

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

type WorkspaceTab = "configure" | "topology" | "files";

/**
 * The environment editor: pick technologies/capabilities and tune knobs, and
 * inspect the live-planned topology diagram and rendered files.
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
  const env = useEnvironment(seededCatalog);
  const { catalog, selection, plan, error, planning } = env;
  const [selectedNodeId, setSelectedNodeId] = useState<string | undefined>();
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>("configure");

  const graph = plan?.graph;

  return (
    <TooltipProvider>
      <div className="flex h-full min-h-0 flex-col p-3">
        <Tabs
          value={workspaceTab}
          onValueChange={(value) => setWorkspaceTab(value as WorkspaceTab)}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div className="mb-3 flex items-center gap-3">
            <TabsList>
              <TabsTrigger value="configure">Configure</TabsTrigger>
              <TabsTrigger value="topology">Topology</TabsTrigger>
              <TabsTrigger value="files">Files</TabsTrigger>
            </TabsList>
            {planning && (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Planning environment…
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
                onToggleModule={env.toggleModule}
                onToggleCapability={env.toggleCapability}
                onSetKnob={env.setKnob}
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
                  Choose technologies on the Configure tab to see the
                  environment topology.
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
