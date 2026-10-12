import { SelectionStep } from "../selection/SelectionStep";
import type { CatalogDto, Selection } from "../types";

export interface ConfigPanelProps {
  catalog: CatalogDto | undefined;
  selection: Selection;
  onToggleModule: (moduleId: string) => void;
  onToggleCapability: (capability: string) => void;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
}

/**
 * Technology/capability selection and inline knobs: the list view of the same
 * selection the canvas edits. Every change replans live.
 */
export function ConfigPanel({
  catalog,
  selection,
  onToggleModule,
  onToggleCapability,
  onSetKnob,
}: ConfigPanelProps) {
  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <div className="w-full max-w-3xl">
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
    </div>
  );
}
