import { Button } from "@open-lakehouse/ui-kit";
import { Loader2, Play } from "lucide-react";
import { SelectionStep } from "../selection/SelectionStep";
import type { CatalogDto, Selection } from "../types";

export interface ConfigPanelProps {
  catalog: CatalogDto | undefined;
  selection: Selection;
  generating: boolean;
  canGenerate: boolean;
  onToggleModule: (moduleId: string) => void;
  onToggleCapability: (capability: string) => void;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
  onGenerate: () => void;
}

/** Technology/capability selection, inline knobs, and the generate action. */
export function ConfigPanel({
  catalog,
  selection,
  generating,
  canGenerate,
  onToggleModule,
  onToggleCapability,
  onSetKnob,
  onGenerate,
}: ConfigPanelProps) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 overflow-auto p-4">
        {catalog ? (
          <SelectionStep
            catalog={catalog}
            selectedModules={selection.modules}
            selectedCapabilities={selection.capabilities}
            overrides={selection.knob_overrides}
            onToggleModule={onToggleModule}
            onToggleCapability={onToggleCapability}
            onSetKnob={onSetKnob}
          />
        ) : (
          <p className="text-sm text-muted-foreground">Loading catalog…</p>
        )}
      </div>
      <div className="shrink-0 border-t bg-background p-4">
        <Button
          type="button"
          onClick={onGenerate}
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
  );
}
