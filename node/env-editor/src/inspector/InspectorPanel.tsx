import {
  Button,
  cn,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@open-lakehouse/ui-kit";
import {
  ArrowLeft,
  ArrowRight,
  FileText,
  Lock,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import {
  type FormEvent,
  type ReactNode,
  type Ref,
  useEffect,
  useRef,
  useState,
} from "react";
import { ModuleIcon } from "../graph/projectIcons";
import { roleStyle } from "../graph/roleTheme";
import { KnobField } from "../knobs/KnobField";
import { describeOrigin, type Origin } from "../state/origin";
import type {
  CatalogDto,
  ExternalResource,
  GatewaySettings,
  ModuleDto,
  PlanResult,
  ProvisionedDto,
  ResourceNounDto,
  TopologyEdgeDto,
} from "../types";
import { AccessSection } from "./AccessSection";
import { canRunExternally, ExternalSection } from "./ExternalSection";

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
  /** Bump to scroll the resources into view and focus the add field. */
  resourcesSeq: number;
  /** Provision an extra database / bucket of `kind` on this component. */
  onAddResource: (moduleId: string, kind: string, name: string) => void;
  /** Drop an extra resource. */
  onRemoveResource: (kind: string, name: string) => void;
  /** How this provider is provided by an external cloud, if it is. */
  external: ExternalResource | undefined;
  /** Mark this provider external (or run it in the stack: `undefined`). */
  onSetExternal: (
    moduleId: string,
    external: ExternalResource | undefined,
  ) => void;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
  /** How the gateway guards access; shown when the gateway is inspected. */
  gateway: GatewaySettings | undefined;
  onSetGateway: (gateway: GatewaySettings) => void;
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
  resourcesSeq,
  onAddResource,
  onRemoveResource,
  external,
  onSetExternal,
  onSetKnob,
  gateway,
  onSetGateway,
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
  const resourceKinds = module?.resource_kinds ?? [];
  const provisions = node?.provisions ?? [];
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

        {module && canRunExternally(module) && (
          <ExternalSection
            module={module}
            external={external}
            onChange={(next) => onSetExternal(moduleId, next)}
          />
        )}

        {role === "gateway" && (
          <Section title="Access">
            <AccessSection gateway={gateway} onChange={onSetGateway} />
          </Section>
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

        {resourceKinds.map((kind) => (
          <ResourcesSection
            key={kind}
            kind={kind}
            noun={module?.resource_nouns.find((n) => n.kind === kind)}
            items={provisions.filter((p) => p.resource === kind)}
            names={names}
            focusSeq={kind === resourceKinds[0] ? resourcesSeq : 0}
            onAdd={(name) => onAddResource(moduleId, kind, name)}
            onRemove={(name) => onRemoveResource(kind, name)}
          />
        ))}

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
            {describeOrigin(origin)}.{" "}
            {origin.kind === "gateway"
              ? origin.role === "gateway"
                ? "It can't be removed."
                : "Change it in the gateway's Access settings."
              : "It leaves the environment with what needs it."}
          </p>
        )}
      </div>
    </aside>
  );
}

/** What a resource kind is called, and what a valid name looks like. */
const RESOURCE_KIND: Record<
  string,
  { plural: string; singular: string; pattern: RegExp; rule: string }
> = {
  relational_db: {
    plural: "Databases",
    singular: "database",
    pattern: /^[a-z_][a-z0-9_]{0,62}$/,
    rule: "lowercase letters, digits and _, starting with a letter or _",
  },
  object_store: {
    plural: "Buckets",
    singular: "bucket",
    pattern: /^(?!.*--)[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$/,
    rule: "3–63 lowercase letters, digits and -, starting and ending alphanumeric, with no consecutive -",
  },
};
const FALLBACK_KIND = {
  plural: "Resources",
  singular: "resource",
  pattern: /^[a-z0-9][a-z0-9_-]{0,62}$/,
  rule: "lowercase letters, digits, _ and -",
};

/** One resource kind the component provisions: what's there and who uses
 *  it, with a field to add another. */
function ResourcesSection({
  kind,
  noun,
  items,
  names,
  focusSeq,
  onAdd,
  onRemove,
}: {
  kind: string;
  /** The provider's own name for this kind (e.g. Azurite's containers). */
  noun?: ResourceNounDto;
  items: ProvisionedDto[];
  names: Map<string, string>;
  focusSeq: number;
  onAdd: (name: string) => void;
  onRemove: (name: string) => void;
}) {
  const base = RESOURCE_KIND[kind] ?? FALLBACK_KIND;
  const meta = noun
    ? {
        ...base,
        singular: noun.singular,
        plural: noun.plural.charAt(0).toUpperCase() + noun.plural.slice(1),
      }
    : base;
  const [draft, setDraft] = useState("");
  const sectionRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focusSeq > 0) {
      sectionRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      });
      inputRef.current?.focus();
    }
  }, [focusSeq]);

  const name = draft.trim();
  const problem =
    name === ""
      ? null
      : items.some((i) => i.name === name)
        ? `This ${meta.singular} already exists.`
        : meta.pattern.test(name)
          ? null
          : `Use ${meta.rule}.`;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (name === "" || problem) return;
    onAdd(name);
    setDraft("");
  };

  return (
    <Section ref={sectionRef} title={meta.plural}>
      {items.length > 0 && (
        <ul className="mb-2 flex flex-col">
          {items.map((item) => (
            <li
              key={item.name}
              className="group flex items-center gap-2 rounded-sm px-1 py-1 text-xs"
            >
              <span className="min-w-0 flex-1 truncate font-mono">
                {item.name}
              </span>
              <span className="truncate text-[11px] text-muted-foreground">
                {item.consumers.length > 0
                  ? `Used by ${item.consumers.map((c) => names.get(c) ?? c).join(", ")}`
                  : "Added by you"}
              </span>
              {item.consumers.length === 0 && (
                <button
                  type="button"
                  aria-label={`Remove ${item.name}`}
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => onRemove(item.name)}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={submit} className="flex flex-col gap-1">
        <div className="flex gap-2">
          <Input
            ref={inputRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={`New ${meta.singular} name`}
            aria-label={`New ${meta.singular} name`}
            aria-invalid={!!problem}
            className="h-8 font-mono text-xs"
          />
          <Button
            type="submit"
            size="sm"
            variant="outline"
            className="h-8"
            disabled={name === "" || !!problem}
          >
            <Plus className="h-3.5 w-3.5" />
            Add
          </Button>
        </div>
        {problem && <p className="text-[11px] text-destructive">{problem}</p>}
        <p className="text-[11px] text-muted-foreground">
          Created by the init job, for use outside the stack.
        </p>
      </form>
    </Section>
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
