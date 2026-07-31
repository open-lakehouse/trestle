import { Badge, Card, cn, Switch } from "@open-lakehouse/ui-kit";
import { Boxes } from "lucide-react";
import { KnobField } from "../knobs/KnobField";
import type { CatalogDto, ModuleDto } from "../types";

export interface TechnologyPickerProps {
  catalog: CatalogDto;
  /** Currently-selected module ids. */
  selected: string[];
  /** module id → (knob key → value) overrides. */
  overrides: Record<string, Record<string, string>>;
  /** Toggle a module in/out of the selection. */
  onToggle: (moduleId: string) => void;
  /** Set one knob override. */
  onSetKnob: (moduleId: string, key: string, value: string) => void;
}

/** Group modules by their `category` (uncategorized last), preserving order. */
function groupByCategory(modules: ModuleDto[]): [string, ModuleDto[]][] {
  const groups = new Map<string, ModuleDto[]>();
  for (const m of modules) {
    const key = m.category ?? "other";
    const list = groups.get(key) ?? [];
    list.push(m);
    groups.set(key, list);
  }
  return [...groups.entries()].sort(([a], [b]) =>
    a === "other" ? 1 : b === "other" ? -1 : a.localeCompare(b),
  );
}

/**
 * Pick and configure technologies in a compact list grouped by category. Each
 * row combines the module summary and selection toggle with its knobs.
 */
export function TechnologyPicker({
  catalog,
  selected,
  overrides,
  onToggle,
  onSetKnob,
}: TechnologyPickerProps) {
  const selectedSet = new Set(selected);
  // A module conflicts if any currently-selected module lists it (or it lists a
  // selected one) in conflicts_with.
  const conflicting = new Set<string>();
  for (const m of catalog.modules) {
    if (!selectedSet.has(m.id)) continue;
    for (const c of m.conflicts_with) conflicting.add(c);
  }
  for (const m of catalog.modules) {
    if (m.conflicts_with.some((c) => selectedSet.has(c))) conflicting.add(m.id);
  }

  return (
    <div className="flex flex-col gap-6">
      {groupByCategory(catalog.modules).map(([category, modules]) => (
        <section key={category} className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {category}
          </h3>
          <div className="flex flex-col gap-2">
            {modules.map((m) => {
              const isSelected = selectedSet.has(m.id);
              const isConflicting = !isSelected && conflicting.has(m.id);
              const toggleId = `module-${m.id}`;
              return (
                <Card
                  key={m.id}
                  className={cn(
                    "overflow-hidden transition-colors",
                    isSelected && "border-primary",
                    isConflicting && "opacity-60",
                  )}
                >
                  <div className="flex items-start gap-3 p-3">
                    <Boxes className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <label
                        htmlFor={toggleId}
                        className="block cursor-pointer text-sm font-semibold text-card-foreground"
                      >
                        {m.display_name ?? m.id}
                      </label>
                      {m.summary && (
                        <div className="mt-0.5 text-xs text-muted-foreground">
                          {m.summary}
                        </div>
                      )}
                      <div className="mt-1.5 flex flex-wrap gap-1">
                        {m.provider_of && (
                          <Badge variant="primary">{m.provider_of}</Badge>
                        )}
                        {m.requires.length > 0 && (
                          <Badge variant="outline">
                            needs {m.requires.length}
                          </Badge>
                        )}
                        {isConflicting && (
                          <Badge variant="destructive">conflict</Badge>
                        )}
                      </div>
                    </div>
                    <Switch
                      id={toggleId}
                      checked={isSelected}
                      onCheckedChange={() => onToggle(m.id)}
                      aria-label={`Enable ${m.display_name ?? m.id}`}
                      className="shrink-0"
                    />
                  </div>
                  {isSelected && m.knobs.length > 0 && (
                    <div className="border-t bg-muted/20 px-3 py-4">
                      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                        {m.knobs.map((knob) => (
                          <KnobField
                            key={knob.key}
                            fieldId={`${m.id}.${knob.key}`}
                            knob={knob}
                            value={overrides[m.id]?.[knob.key]}
                            onChange={(value) =>
                              onSetKnob(m.id, knob.key, value)
                            }
                          />
                        ))}
                      </div>
                    </div>
                  )}
                </Card>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
