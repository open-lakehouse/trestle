import { Badge, Card, cn } from "@open-lakehouse/ui-kit";
import { Boxes } from "lucide-react";
import type { CatalogDto, ModuleDto } from "../types";

export interface TechnologyPickerProps {
  catalog: CatalogDto;
  /** Currently-selected module ids. */
  selected: string[];
  /** Toggle a module in/out of the selection. */
  onToggle: (moduleId: string) => void;
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
 * Pick technologies directly (module cards grouped by category). A card shows
 * the module's name/summary, a role-colored marker, and its requires/conflicts
 * hints; conflicting modules are flagged relative to the current selection.
 */
export function TechnologyPicker({
  catalog,
  selected,
  onToggle,
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
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {modules.map((m) => {
              const isSelected = selectedSet.has(m.id);
              const isConflicting = !isSelected && conflicting.has(m.id);
              return (
                <Card
                  key={m.id}
                  role="button"
                  tabIndex={0}
                  aria-pressed={isSelected}
                  onClick={() => onToggle(m.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onToggle(m.id);
                    }
                  }}
                  className={cn(
                    "cursor-pointer p-3 transition-colors hover:border-primary/60",
                    isSelected && "border-primary ring-2 ring-primary/30",
                    isConflicting && "opacity-60",
                  )}
                >
                  <div className="flex items-start gap-2">
                    <Boxes className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm font-semibold text-card-foreground">
                        {m.display_name ?? m.id}
                      </div>
                      {m.summary && (
                        <div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
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
                  </div>
                </Card>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
