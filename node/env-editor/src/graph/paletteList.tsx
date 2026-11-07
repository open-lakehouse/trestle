// The pieces both palettes (⌘K commands, Add module…) are built from: a dialog
// with a search field over grouped rows, arrow keys to move, Enter to pick.

import {
  cn,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
} from "@open-lakehouse/ui-kit";
import { Search } from "lucide-react";
import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";

/** A palette dialog: a search field, then `children` (the groups), then a
 *  line of key hints. */
export function PaletteDialog({
  open,
  onOpenChange,
  title,
  description,
  query,
  onQueryChange,
  onKeyDown,
  placeholder,
  listRef,
  above,
  hints,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  query: string;
  onQueryChange: (query: string) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  placeholder: string;
  listRef: React.RefObject<HTMLDivElement | null>;
  /** Drawn between the search field and the list (e.g. filters). */
  above?: ReactNode;
  hints: readonly string[];
  children: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-[15%] max-w-lg translate-y-0 gap-0 overflow-hidden">
        <DialogTitle className="sr-only">{title}</DialogTitle>
        <DialogDescription className="sr-only">{description}</DialogDescription>
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            autoFocus
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={placeholder}
            className="h-11 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
          />
        </div>
        {above}
        <div ref={listRef} className="max-h-[60vh] overflow-y-auto p-1">
          {children}
        </div>
        <div className="flex items-center gap-3 border-t border-border px-3 py-2 text-[11px] text-muted-foreground">
          {hints.map((h) => (
            <span key={h}>{h}</span>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function PaletteGroup({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="py-1">
      <div className="px-2 pb-1 pt-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      {children}
    </div>
  );
}

/** One pickable (or disabled) row. */
export function PaletteRow({
  itemKey,
  active,
  disabled,
  onPick,
  onHover,
  icon,
  title,
  subtitle,
  trailing,
  trailingClassName,
}: {
  itemKey: string;
  active: boolean;
  disabled?: boolean;
  onPick: () => void;
  onHover: () => void;
  icon?: ReactNode;
  title: string;
  subtitle?: string;
  trailing?: ReactNode;
  trailingClassName?: string;
}) {
  return (
    <button
      type="button"
      data-key={itemKey}
      aria-current={active || undefined}
      disabled={disabled}
      onClick={onPick}
      onMouseMove={onHover}
      className={cn(
        // The top scroll margin keeps a group's title visible when scrolling
        // up onto its first row.
        "flex w-full scroll-mb-1 scroll-mt-8 items-center gap-3 rounded-md px-2 py-2 text-left transition-colors",
        active
          ? "bg-primary/[0.07] ring-1 ring-inset ring-primary/25"
          : !disabled && "hover:bg-muted/60",
        disabled && "cursor-default opacity-55",
      )}
    >
      {icon !== undefined && (
        <div
          className={cn(
            "flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
            active ? "bg-background" : "bg-muted",
          )}
        >
          {icon}
        </div>
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-card-foreground">
          {title}
        </div>
        {subtitle && (
          <div className="truncate text-xs text-muted-foreground">
            {subtitle}
          </div>
        )}
      </div>
      {trailing !== undefined && (
        <span
          className={cn(
            "max-w-[45%] shrink-0 truncate text-[11px] text-muted-foreground",
            trailingClassName,
          )}
        >
          {trailing}
        </span>
      )}
    </button>
  );
}

/**
 * Keyboard navigation over a palette's pickable rows (by key): the
 * highlighted one, ↑/↓ to move it, Enter to pick it, and the pointer to move
 * it. `fallback` is the index highlighted until the user moves.
 */
export function usePaletteNav<T>({
  open,
  items,
  keyOf,
  fallback = 0,
  onPick,
}: {
  open: boolean;
  items: readonly T[];
  keyOf: (item: T) => string;
  fallback?: number;
  onPick: (item: T) => void;
}) {
  /** The highlighted row; `null` follows the fallback. */
  const [active, setActive] = useState<number | null>(null);
  useEffect(() => {
    if (open) setActive(null);
  }, [open]);
  const current = Math.min(active ?? fallback, items.length - 1);
  const activeItem = current >= 0 ? items[current] : undefined;
  const activeKey = activeItem === undefined ? undefined : keyOf(activeItem);

  // Keep the highlighted row in view as the arrow keys move it. Only for the
  // keyboard: scrolling a half-visible row under the pointer would jitter.
  const listRef = useRef<HTMLDivElement>(null);
  const movedByKey = useRef(false);
  useEffect(() => {
    if (!activeKey || !movedByKey.current) return;
    listRef.current
      ?.querySelector(`[data-key="${CSS.escape(activeKey)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeKey]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      movedByKey.current = true;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive(
        items.length === 0
          ? null
          : (current + step + items.length) % items.length,
      );
    } else if (e.key === "Enter" && activeItem !== undefined) {
      e.preventDefault();
      onPick(activeItem);
    }
  };

  const hover = (item: T) => {
    const i = items.indexOf(item);
    if (i < 0 || i === current) return;
    movedByKey.current = false;
    setActive(i);
  };

  return {
    activeKey,
    listRef,
    onKeyDown,
    hover,
    reset: () => setActive(null),
  };
}
