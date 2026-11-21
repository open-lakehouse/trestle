// Selections saved before a catalog module was renamed. The planner migrates
// what it plans (`migrate_selection` in crates/stack-topology/src/manifest.rs),
// but only a copy: the editor edits by node id, so it adopts the rename itself
// wherever a stored selection comes in. Mirrors the Rust function; keep the two
// in step.

import type {
  FeatureChoice,
  HostedResource,
  PlatformFeature,
  PlatformSelection,
  Selection,
} from "../types.js";

/** Unity Catalog's id before it was split by implementation (manifest v6). */
const LEGACY_UNITY_CATALOG = "unity-catalog";
/** The open-source server: what `unity-catalog` ran in a container. */
const UNITY_CATALOG_OSS = "unity-catalog-oss";
/** mangrove's server: what `unity-catalog` ran as a Neon Function. */
const UNITY_CATALOG_RS = "unity-catalog-rs";

/**
 * `selection` with renamed module ids rewritten, or `selection` itself when
 * nothing changed (so a caller can tell by identity).
 *
 * `unity-catalog` becomes `unity-catalog-oss` with its knob overrides, unless
 * it ran as a function: then `unity-catalog-rs`, keeping the function entry
 * and dropping the overrides (they were the OSS server's). When the new id is
 * there too, it is selected once, and its own settings win over the legacy
 * ones, which only fill what it leaves unset.
 */
export function migrateSelection(selection: Selection): Selection {
  return migratePlatform(migrateUnityCatalog(selection));
}

function migrateUnityCatalog(selection: Selection): Selection {
  const legacy = LEGACY_UNITY_CATALOG;
  const selected = selection.modules.includes(legacy);
  const hosted = selection.hosted?.[legacy];
  const overrides = selection.knob_overrides[legacy];
  if (!selected && !hosted && !overrides) return selection;

  const to =
    hosted?.connection.kind === "function"
      ? UNITY_CATALOG_RS
      : UNITY_CATALOG_OSS;
  const modules = [
    ...new Set(selection.modules.map((m) => (m === legacy ? to : m))),
  ];

  const { [legacy]: _hosted, ...otherHosted } = selection.hosted ?? {};
  const nextHosted: Record<string, HostedResource> = otherHosted;
  if (hosted && !nextHosted[to]) nextHosted[to] = hosted;

  const { [legacy]: _knobs, ...knob_overrides } = selection.knob_overrides;
  if (overrides && to === UNITY_CATALOG_OSS) {
    knob_overrides[to] = { ...overrides, ...knob_overrides[to] };
  }

  const next: Selection = { ...selection, modules, knob_overrides };
  if (selection.hosted) next.hosted = nextHosted;
  return next;
}

/** The baseline gateway, whose `auth` knob turned on sign-in before manifest
 *  v7. */
const LEGACY_GATEWAY = "envoy";
/** The gateway knob keys that turned on sign-in before manifest v7. */
const LEGACY_AUTH_KNOBS = ["auth", "ENVOY_AUTH"];
/** The baseline providers a selection listed as modules to turn their feature
 *  on, before manifest v7. */
const LEGACY_PROVIDERS: Record<string, PlatformFeature> = {
  authelia: "identity",
  opa: "policy",
  jaeger: "observability",
};
/** The capabilities that turned a feature on, before manifest v7. */
const LEGACY_CAPABILITIES: Record<string, PlatformFeature> = {
  auth: "identity",
  policy: "policy",
  tracing: "observability",
};

/**
 * `selection` with its pre-v7 sign-in, policy and tracing settings moved to
 * platform features, or `selection` itself when there are none. Mirrors
 * `migrate_platform` in crates/stack-topology/src/manifest.rs.
 *
 * `gateway.auth` / `gateway.policy`, the listed baseline providers (`authelia`,
 * `opa`, `jaeger`) and the `auth` / `policy` / `tracing` capabilities turn
 * their features on (a policy engine implied sign-in). The gateway's `auth`
 * knob turns identity on, or with `false` keeps it off (dropping a listed
 * provider). A feature already set keeps its provider.
 */
export function migratePlatform(selection: Selection): Selection {
  const on = new Map<PlatformFeature, FeatureChoice>();
  const turnOn = (feature: PlatformFeature, choice: FeatureChoice) => {
    if (!on.has(feature)) on.set(feature, choice);
  };
  let changed = false;
  const gateway = selection.gateway ? { ...selection.gateway } : undefined;
  if (gateway?.auth) {
    turnOn("identity", true);
    changed = true;
  }
  if (gateway?.policy) {
    turnOn("policy", true);
    turnOn("identity", true);
    changed = true;
  }
  if (gateway) {
    delete gateway.auth;
    delete gateway.policy;
  }
  const modules = selection.modules.filter((m) => {
    const feature = LEGACY_PROVIDERS[m];
    if (!feature) return true;
    turnOn(feature, m);
    if (feature === "policy") turnOn("identity", true);
    changed = true;
    return false;
  });
  const capabilities = selection.capabilities.filter((c) => {
    const feature = LEGACY_CAPABILITIES[c];
    if (!feature) return true;
    turnOn(feature, true);
    if (feature === "policy") turnOn("identity", true);
    changed = true;
    return false;
  });
  let knob_overrides = selection.knob_overrides;
  const gatewayKnobs = knob_overrides[LEGACY_GATEWAY];
  const knobKey = LEGACY_AUTH_KNOBS.find(
    (k) => gatewayKnobs?.[k] !== undefined,
  );
  if (gatewayKnobs && knobKey) {
    const raw = gatewayKnobs[knobKey].trim().toLowerCase();
    const rest = Object.fromEntries(
      Object.entries(gatewayKnobs).filter(
        ([k]) => !LEGACY_AUTH_KNOBS.includes(k),
      ),
    );
    const { [LEGACY_GATEWAY]: _gateway, ...others } = knob_overrides;
    knob_overrides = Object.keys(rest).length
      ? { ...others, [LEGACY_GATEWAY]: rest }
      : others;
    if (["true", "1", "yes", "on"].includes(raw)) turnOn("identity", true);
    if (["false", "0", "no", "off"].includes(raw)) on.delete("identity");
    changed = true;
  }
  if (!changed) return selection;

  const platform: PlatformSelection = { ...selection.platform };
  for (const [feature, choice] of on) platform[feature] ??= choice;
  const next: Selection = {
    ...selection,
    modules,
    capabilities,
    knob_overrides,
    platform,
  };
  if (gateway) {
    if (gateway.enabled === false) next.gateway = gateway;
    else delete next.gateway;
  }
  return next;
}
