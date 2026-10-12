import {
  Button,
  cn,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@open-lakehouse/ui-kit";
import { ArrowLeft, ArrowRight, FileText, Lock, Trash2, X } from "lucide-react";
import { type ReactNode, type Ref, useEffect, useRef } from "react";
import { ModuleIcon } from "../graph/projectIcons";
import { roleStyle } from "../graph/roleTheme";
import { KnobField } from "../knobs/KnobField";
import { describeOrigin, type Origin } from "../state/origin";
import type {
  CatalogDto,
  ModuleDto,
  PlanResult,
  TopologyEdgeDto,
} from "../types";

export interface InspectorPanelProps {
  /** The inspected component's module id. */
  moduleId: string;
  catalog: CatalogDto | undefined;
  plan: PlanResult | null;
  /** module id → (knob key → value) overrides. */
  overrides: Record<string, Record<string, string>>;
  origin: Origin;
  alternatives: ModuleDto[];
  /** Bump to scroll the settings into view (the "configure" action). */
  settingsSeq: number;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
  onRemove: (moduleId: string) => void;
  onSwap: (fromId: string, toId: string) => void;
  /** Navigate to a connected component. */
  onSelect: (moduleId: string) => void;
  /** Open a rendered file in the Files view. */
  onOpenFile: (path: string) => void;
  onClose: () => void;
}

/**
 * The docked side panel for the selected component: what it is and why it's
 * here, its settings, what it connects to, the files it renders, and swap /
 * remove. Works from the catalog alone, so a just-added module is editable
 * before its replan lands.
 */
export function InspectorPanel({
  moduleId,
  catalog,
  plan,
  overrides,
  origin,
  alternatives,
  settingsSeq,
  onSetKnob,
  onRemove,
  onSwap,
  onSelect,
  onOpenFile,
  onClose,
}: InspectorPanelProps) {
  const module = catalog?.modules.find((m) => m.id === moduleId);
  const node = plan?.graph.nodes.find((n) => n.id === moduleId);
  const title = node?.display_name ?? module?.display_name ?? moduleId;
  const role = node?.role ?? module?.provider_of;
  const summary = node?.summary ?? module?.summary;
  const knobs = module?.knobs ?? [];
  const names = new Map(
    plan?.graph.nodes.map((n) => [n.id, n.display_name ?? n.id]) ?? [],
  );
  const connections = (plan?.graph.edges ?? []).flatMap((e) =>
    describeConnection(e, moduleId, (id) => names.get(id) ?? id),
  );
  const files = (plan?.files ?? []).filter((f) =>
    f.path.startsWith(`modules/${moduleId}/`),
  );

  const settingsRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (settingsSeq > 0) {
      settingsRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      });
    }
  }, [settingsSeq]);

  return (
    <aside className="flex h-full w-[340px] shrink-0 flex-col border-l border-border bg-card">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <div
          className={cn(
            "flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-current/10",
            roleStyle(role).accent,
          )}
        >
          <ModuleIcon moduleId={moduleId} role={role} className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold text-card-foreground">
            {title}
          </h2>
          <div className="text-xs text-muted-foreground">
            {roleStyle(role).label} · {describeOrigin(origin)}
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="-mr-2 -mt-1 h-7 w-7 shrink-0"
          aria-label="Close inspector"
          onClick={onClose}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {summary && (
          <p className="px-4 pt-3 text-xs text-muted-foreground">{summary}</p>
        )}

        <Section ref={settingsRef} title="Settings">
          {knobs.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              This component has no settings.
            </p>
          ) : (
            <div className="flex flex-col gap-3">
              {knobs.map((knob) => (
                <KnobField
                  key={knob.key}
                  fieldId={`inspector.${moduleId}.${knob.key}`}
                  knob={knob}
                  value={overrides[moduleId]?.[knob.key]}
                  onChange={(value) => onSetKnob(moduleId, knob.key, value)}
                />
              ))}
            </div>
          )}
        </Section>

        {connections.length > 0 && (
          <Section title="Connections">
            <ul className="flex flex-col">
              {connections.map((c) => (
                <li key={c.key}>
                  <button
                    type="button"
                    disabled={!c.target}
                    onClick={() => c.target && onSelect(c.target)}
                    className="flex w-full items-center gap-2 rounded-sm px-1 py-1 text-left text-xs enabled:hover:bg-muted"
                  >
                    {c.outgoing ? (
                      <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                    ) : (
                      <ArrowLeft className="h-3 w-3 shrink-0 text-muted-foreground" />
                    )}
                    <span className="min-w-0 flex-1 truncate">{c.text}</span>
                  </button>
                </li>
              ))}
            </ul>
          </Section>
        )}

        {files.length > 0 && (
          <Section title="Files">
            <ul className="flex flex-col">
              {files.map((f) => (
                <li key={f.path}>
                  <button
                    type="button"
                    onClick={() => onOpenFile(f.path)}
                    className="flex w-full items-center gap-2 rounded-sm px-1 py-1 text-left font-mono text-[11px] hover:bg-muted"
                  >
                    <FileText className="h-3 w-3 shrink-0 text-muted-foreground" />
                    <span className="truncate">
                      {f.path.slice(`modules/${moduleId}/`.length)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </Section>
        )}
      </div>

      <div className="flex flex-col gap-2 border-t border-border p-4">
        {alternatives.length > 0 && (
          <Select value="" onValueChange={(to) => onSwap(moduleId, to)}>
            <SelectTrigger aria-label="Replace with another module">
              <SelectValue placeholder="Replace with…" />
            </SelectTrigger>
            <SelectContent>
              {alternatives.map((m) => (
                <SelectItem key={m.id} value={m.id}>
                  {m.display_name ?? m.id}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
        {origin.kind === "explicit" ? (
          <Button
            type="button"
            variant="outline"
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            onClick={() => onRemove(moduleId)}
          >
            <Trash2 className="h-4 w-4" />
            Remove from environment
          </Button>
        ) : (
          <p className="flex items-start gap-2 text-xs text-muted-foreground">
            <Lock className="mt-0.5 h-3 w-3 shrink-0" />
            {describeOrigin(origin)}. It leaves the environment with what needs
            it.
          </p>
        )}
      </div>
    </aside>
  );
}

function Section({
  ref,
  title,
  children,
}: {
  ref?: Ref<HTMLElement>;
  title: string;
  children: ReactNode;
}) {
  return (
    <section ref={ref} className="scroll-mt-2 px-4 py-3">
      <h3 className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      {children}
    </section>
  );
}

interface Connection {
  key: string;
  text: string;
  outgoing: boolean;
  /** The component at the other end, if it can be inspected. */
  target?: string;
}

/** Describe an edge from `id`'s side, or nothing if it isn't incident. */
function describeConnection(
  e: TopologyEdgeDto,
  id: string,
  name: (id: string) => string,
): Connection[] {
  const outgoing = e.from === id;
  if (!outgoing && e.to !== id) return [];
  const other = outgoing ? e.to : e.from;
  const key = `${e.kind}:${e.from}:${e.to}:${"host_port" in e ? e.host_port : ""}`;
  const target = e.kind === "access" ? undefined : other;
  const text = (() => {
    switch (e.kind) {
      case "access":
        return `Clients reach it on :${e.host_port}`;
      case "route":
        return outgoing
          ? `Exposes ${name(e.to)} on :${e.host_port}${e.gated ? " (sign-in)" : ""}`
          : `Exposed by ${name(e.from)} on :${e.host_port}`;
      case "authenticates":
        return outgoing
          ? `Signs users in with ${name(e.to)}`
          : `Signs users in for ${name(e.from)}`;
      case "authorizes":
        return outgoing
          ? `Checks policy with ${name(e.to)}`
          : `Authorizes requests for ${name(e.from)}`;
      case "uses": {
        const what =
          e.resources.length > 0
            ? e.resources.join(", ")
            : roleStyle(e.role).label.toLowerCase();
        return outgoing
          ? `Uses ${name(e.to)} for ${what}`
          : `Used by ${name(e.from)} for ${what}`;
      }
    }
  })();
  return [{ key, text, outgoing, target }];
}
