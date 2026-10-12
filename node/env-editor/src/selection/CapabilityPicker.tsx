import { Badge, Card, cn } from "@open-lakehouse/ui-kit";
import { Sparkles } from "lucide-react";
import type { CatalogDto } from "../types";
import { capabilityIndex, capabilityLabel } from "./catalogIndex";

export interface CapabilityPickerProps {
  catalog: CatalogDto;
  /** Currently-selected capability names. */
  selected: string[];
  /** Toggle a capability in/out of the selection. */
  onToggle: (capability: string) => void;
}

/**
 * Pick by capability ("I want experiment tracking"). Each card is one capability
 * the catalog can satisfy, showing which module(s) provide it. The planner maps
 * a selected capability to its provider module(s).
 */
export function CapabilityPicker({
  catalog,
  selected,
  onToggle,
}: CapabilityPickerProps) {
  const selectedSet = new Set(selected);
  const capabilities = capabilityIndex(catalog);

  if (capabilities.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No capabilities are declared in this catalog.
      </p>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
      {capabilities.map(([capability, providers]) => {
        const isSelected = selectedSet.has(capability);
        return (
          <Card
            key={capability}
            role="button"
            tabIndex={0}
            aria-pressed={isSelected}
            onClick={() => onToggle(capability)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onToggle(capability);
              }
            }}
            className={cn(
              "cursor-pointer p-3 transition-colors hover:border-primary/60",
              isSelected && "border-primary ring-2 ring-primary/30",
            )}
          >
            <div className="flex items-start gap-2">
              <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-semibold text-card-foreground">
                  {capabilityLabel(capability)}
                </div>
                <div className="mt-1 flex flex-wrap gap-1">
                  {providers.map((p) => (
                    <Badge key={p.id} variant="outline">
                      {p.display_name ?? p.id}
                    </Badge>
                  ))}
                </div>
              </div>
            </div>
          </Card>
        );
      })}
    </div>
  );
}
