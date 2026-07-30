import { Card } from "@open-lakehouse/ui-kit";
import type { CatalogDto } from "../types";
import { KnobField } from "./KnobField";

export interface KnobsStepProps {
  catalog: CatalogDto;
  /** The selected module ids whose knobs to show. */
  selectedModules: string[];
  /** module id → (knob key → value) overrides. */
  overrides: Record<string, Record<string, string>>;
  /** Set one knob override. */
  onSetKnob: (moduleId: string, key: string, value: string) => void;
}

/**
 * A form generated from the selected modules' knobs — one section per module
 * that exposes any. Writes into `Selection.knob_overrides`.
 */
export function KnobsStep({
  catalog,
  selectedModules,
  overrides,
  onSetKnob,
}: KnobsStepProps) {
  const selectedSet = new Set(selectedModules);
  const modulesWithKnobs = catalog.modules.filter(
    (m) => selectedSet.has(m.id) && m.knobs.length > 0,
  );

  if (modulesWithKnobs.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        The selected modules expose no configurable knobs.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {modulesWithKnobs.map((m) => (
        <Card key={m.id} className="p-4">
          <h3 className="mb-3 text-sm font-semibold text-card-foreground">
            {m.display_name ?? m.id}
          </h3>
          <div className="flex flex-col gap-3">
            {m.knobs.map((knob) => (
              <KnobField
                key={knob.key}
                fieldId={`${m.id}.${knob.key}`}
                knob={knob}
                value={overrides[m.id]?.[knob.key]}
                onChange={(value) => onSetKnob(m.id, knob.key, value)}
              />
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}
