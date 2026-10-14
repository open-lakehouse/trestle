// The editor's working state: the selection the user is shaping (with undo
// history), the catalog it draws from, and the plan of it. Planning is live:
// every edit replans after a short debounce, and a failed plan keeps the last
// good one on screen next to the error, so the diagram never blanks mid-edit.

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { usePlanner } from "../planner";
import type {
  CatalogDto,
  ExternalResource,
  GatewaySettings,
  PlanResult,
  Selection,
} from "../types";

const EMPTY_SELECTION: Selection = {
  modules: [],
  capabilities: [],
  knob_overrides: {},
};

/** How long edits settle before replanning (typing into a knob, quick toggles). */
const PLAN_DEBOUNCE_MS = 150;
const HISTORY_LIMIT = 100;

export interface EnvironmentState {
  /** The selectable catalog; `undefined` until it loads. */
  catalog: CatalogDto | undefined;
  /** The working selection. */
  selection: Selection;
  /** The plan of the latest selection that planned successfully. */
  plan: PlanResult | null;
  /** The catalog-load or latest planning error, if any. */
  error: string | null;
  /** A replan is pending or running. */
  planning: boolean;
  /** Add a module to the selection. */
  add: (moduleId: string) => void;
  /** Remove a module (and its knob overrides) from the selection. */
  remove: (moduleId: string) => void;
  /** Replace one module with another that fills the same role. */
  swap: (fromId: string, toId: string) => void;
  toggleModule: (moduleId: string) => void;
  toggleCapability: (capability: string) => void;
  setKnob: (moduleId: string, key: string, value: string) => void;
  /** Provision an extra database / bucket on `provider`. */
  addResource: (provider: string, resource: string, name: string) => void;
  /** Drop an extra resource from the selection. */
  removeResource: (resource: string, name: string) => void;
  /** Mark provider `moduleId` as provided by an external cloud (`external`),
   *  or run it in the stack again (`undefined`). */
  setExternal: (
    moduleId: string,
    external: ExternalResource | undefined,
  ) => void;
  /** Set how the gateway guards access (`policy` implies sign-in). */
  setGateway: (gateway: GatewaySettings) => void;
  /** Replace the selection with the catalog's default. */
  applyDefault: () => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
}

interface History {
  past: Selection[];
  present: Selection;
  future: Selection[];
  /** Coalescing key of the last edit, so typing into one knob is one undo step. */
  lastKey?: string;
}

type HistoryAction =
  | { type: "seed"; selection: Selection }
  | { type: "edit"; next: (s: Selection) => Selection; key?: string }
  | { type: "undo" }
  | { type: "redo" };

function historyReducer(h: History, action: HistoryAction): History {
  switch (action.type) {
    case "seed":
      // Seed from the catalog default the first time only.
      return isEmptySelection(h.present) && h.past.length === 0
        ? { past: [], present: cloneSelection(action.selection), future: [] }
        : h;
    case "edit": {
      const next = action.next(h.present);
      if (next === h.present) return h;
      if (action.key && action.key === h.lastKey) {
        return { ...h, present: next, future: [] };
      }
      return {
        past: [...h.past, h.present].slice(-HISTORY_LIMIT),
        present: next,
        future: [],
        lastKey: action.key,
      };
    }
    case "undo": {
      const previous = h.past.at(-1);
      if (!previous) return h;
      return {
        past: h.past.slice(0, -1),
        present: previous,
        future: [h.present, ...h.future],
      };
    }
    case "redo": {
      const [next, ...rest] = h.future;
      if (!next) return h;
      return { past: [...h.past, h.present], present: next, future: rest };
    }
  }
}

/**
 * Own the editor's selection, catalog and live plan, all through the
 * `Planner` seam (from the nearest `PlannerProvider`).
 */
export function useEnvironment(seededCatalog?: CatalogDto): EnvironmentState {
  const planner = usePlanner();
  const [catalog, setCatalog] = useState<CatalogDto | undefined>(seededCatalog);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [history, dispatch] = useReducer(historyReducer, {
    past: [],
    present: EMPTY_SELECTION,
    future: [],
  });
  const selection = history.present;
  const [plan, setPlan] = useState<PlanResult | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [planning, setPlanning] = useState(false);
  /** Monotonic plan run, so a slow stale plan never overwrites a newer one. */
  const runRef = useRef(0);

  useEffect(() => {
    if (seededCatalog) {
      setCatalog(seededCatalog);
      dispatch({ type: "seed", selection: seededCatalog.default_selection });
      return;
    }
    let cancelled = false;
    planner
      .catalog()
      .then((c) => {
        if (cancelled) return;
        setCatalog(c);
        dispatch({ type: "seed", selection: c.default_selection });
      })
      .catch((e) => {
        if (!cancelled) setCatalogError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [planner, seededCatalog]);

  useEffect(() => {
    const run = ++runRef.current;
    if (isEmptySelection(selection)) {
      setPlan(null);
      setPlanError(null);
      setPlanning(false);
      return;
    }
    setPlanning(true);
    const timer = setTimeout(() => {
      planner
        .plan(selection)
        .then((result) => {
          if (run !== runRef.current) return;
          setPlan(result);
          setPlanError(null);
        })
        .catch((e) => {
          if (run === runRef.current) setPlanError(errorMessage(e));
        })
        .finally(() => {
          if (run === runRef.current) setPlanning(false);
        });
    }, PLAN_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [planner, selection]);

  const add = useCallback((moduleId: string) => {
    dispatch({
      type: "edit",
      next: (s) =>
        s.modules.includes(moduleId)
          ? s
          : { ...s, modules: [...s.modules, moduleId] },
    });
  }, []);

  const remove = useCallback((moduleId: string) => {
    dispatch({
      type: "edit",
      next: (s) =>
        s.modules.includes(moduleId)
          ? {
              ...s,
              modules: s.modules.filter((m) => m !== moduleId),
              knob_overrides: withoutKey(s.knob_overrides, moduleId),
              external: withoutKey(s.external ?? {}, moduleId),
            }
          : s,
    });
  }, []);

  const swap = useCallback((fromId: string, toId: string) => {
    dispatch({
      type: "edit",
      next: (s) => ({
        ...s,
        modules: [...s.modules.filter((m) => m !== fromId && m !== toId), toId],
        knob_overrides: withoutKey(s.knob_overrides, fromId),
        external: withoutKey(s.external ?? {}, fromId),
      }),
    });
  }, []);

  const toggleModule = useCallback(
    (moduleId: string) => {
      if (selection.modules.includes(moduleId)) remove(moduleId);
      else add(moduleId);
    },
    [selection, add, remove],
  );

  const toggleCapability = useCallback((capability: string) => {
    dispatch({
      type: "edit",
      next: (s) => ({
        ...s,
        capabilities: s.capabilities.includes(capability)
          ? s.capabilities.filter((c) => c !== capability)
          : [...s.capabilities, capability],
      }),
    });
  }, []);

  const setKnob = useCallback(
    (moduleId: string, key: string, value: string) => {
      dispatch({
        type: "edit",
        key: `knob:${moduleId}.${key}`,
        next: (s) => ({
          ...s,
          knob_overrides: {
            ...s.knob_overrides,
            [moduleId]: { ...s.knob_overrides[moduleId], [key]: value },
          },
        }),
      });
    },
    [],
  );

  const addResource = useCallback(
    (provider: string, resource: string, name: string) => {
      dispatch({
        type: "edit",
        next: (s) => {
          const extras = s.extra_resources ?? [];
          if (extras.some((e) => e.resource === resource && e.name === name)) {
            return s;
          }
          return {
            ...s,
            extra_resources: [...extras, { resource, name, provider }],
          };
        },
      });
    },
    [],
  );

  const removeResource = useCallback((resource: string, name: string) => {
    dispatch({
      type: "edit",
      next: (s) => {
        const extras = s.extra_resources ?? [];
        const kept = extras.filter(
          (e) => !(e.resource === resource && e.name === name),
        );
        return kept.length === extras.length
          ? s
          : { ...s, extra_resources: kept };
      },
    });
  }, []);

  const setExternal = useCallback(
    (moduleId: string, external: ExternalResource | undefined) => {
      dispatch({
        type: "edit",
        // Typing into one field of the form is one undo step; toggling is its own.
        key: external ? `external:${moduleId}` : undefined,
        next: (s) => {
          const current = s.external ?? {};
          if (!external) {
            return moduleId in current
              ? { ...s, external: withoutKey(current, moduleId) }
              : s;
          }
          // Marking a provider external selects it (it stays in the graph).
          return {
            ...s,
            modules: s.modules.includes(moduleId)
              ? s.modules
              : [...s.modules, moduleId],
            external: { ...current, [moduleId]: external },
          };
        },
      });
    },
    [],
  );

  const setGateway = useCallback((gateway: GatewaySettings) => {
    dispatch({
      type: "edit",
      next: (s) => ({ ...s, gateway: { ...gateway } }),
    });
  }, []);

  const applyDefault = useCallback(() => {
    if (!catalog) return;
    dispatch({
      type: "edit",
      next: () => cloneSelection(catalog.default_selection),
    });
  }, [catalog]);

  const undo = useCallback(() => dispatch({ type: "undo" }), []);
  const redo = useCallback(() => dispatch({ type: "redo" }), []);

  return {
    catalog,
    selection,
    plan,
    error: catalogError ?? planError,
    planning,
    add,
    remove,
    swap,
    toggleModule,
    toggleCapability,
    setKnob,
    addResource,
    removeResource,
    setExternal,
    setGateway,
    applyDefault,
    undo,
    redo,
    canUndo: history.past.length > 0,
    canRedo: history.future.length > 0,
  };
}

export function isEmptySelection(selection: Selection): boolean {
  return (
    selection.modules.length === 0 &&
    selection.capabilities.length === 0 &&
    (selection.extra_resources?.length ?? 0) === 0
  );
}

function cloneSelection(selection: Selection): Selection {
  return {
    modules: [...selection.modules],
    capabilities: [...selection.capabilities],
    knob_overrides: { ...selection.knob_overrides },
    extra_resources: [...(selection.extra_resources ?? [])],
    external: { ...(selection.external ?? {}) },
    gateway: selection.gateway && { ...selection.gateway },
  };
}

function withoutKey<T>(record: Record<string, T>, key: string) {
  if (!(key in record)) return record;
  const { [key]: _, ...rest } = record;
  return rest;
}

function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}
