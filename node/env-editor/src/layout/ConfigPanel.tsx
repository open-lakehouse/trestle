import { Card } from "@open-lakehouse/ui-kit";
import { AccessSection } from "../inspector/AccessSection";
import { SelectionStep } from "../selection/SelectionStep";
import type { CatalogDto, GatewaySettings, Selection } from "../types";

export interface ConfigPanelProps {
  catalog: CatalogDto | undefined;
  selection: Selection;
  onToggleModule: (moduleId: string) => void;
  onToggleCapability: (capability: string) => void;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
  onSetGateway: (gateway: GatewaySettings) => void;
}

/**
 * Gateway access, then technology/capability selection and inline knobs: the
 * list view of the same selection the canvas edits. Every change replans live.
 */
export function ConfigPanel({
  catalog,
  selection,
  onToggleModule,
  onToggleCapability,
  onSetKnob,
  onSetGateway,
}: ConfigPanelProps) {
  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <div className="w-full max-w-3xl">
        <Card className="mb-6 p-4">
          <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Gateway access
          </h3>
          <AccessSection gateway={selection.gateway} onChange={onSetGateway} />
        </Card>
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
