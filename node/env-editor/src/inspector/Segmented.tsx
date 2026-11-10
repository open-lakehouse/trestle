import { cn } from "@open-lakehouse/ui-kit";
import type { ReactNode } from "react";

/** One option of a {@link Segmented} control. */
export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

/**
 * A segmented control: a row of mutually exclusive options, as a radio group.
 * Picking the current option does nothing.
 */
export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled = false,
}: {
  /** What is being chosen (the group's accessible name). */
  label: string;
  value: T;
  options: SegmentedOption<T>[];
  onChange: (value: T) => void;
  /** Show the choice without letting it change. */
  disabled?: boolean;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="grid auto-cols-fr grid-flow-col gap-1 rounded-md border border-border bg-muted/40 p-0.5"
    >
      {options.map((o) => {
        const checked = o.value === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a segmented control, not native radio inputs.
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={checked}
            disabled={disabled}
            onClick={() => checked || disabled || onChange(o.value)}
            className={cn(
              "flex h-7 min-w-0 items-center justify-center gap-1.5 rounded-sm px-1.5 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-ring",
              checked
                ? "bg-background font-medium text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
              disabled && "cursor-not-allowed opacity-60",
            )}
          >
            {o.icon}
            <span className="truncate">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
