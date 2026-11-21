// Platform features (identity, policy, observability): environment-wide
// capabilities a selection turns on once, and that every capable module then
// integrates with (mirrors `crates/stack-topology/src/model/platform.rs`).

import type {
  CatalogDto,
  FeatureChoice,
  Integration,
  ModuleDto,
  PlatformFeature,
  Selection,
} from "../types.js";

/** Every platform feature, in the order the editor lists them. */
export const PLATFORM_FEATURES: readonly PlatformFeature[] = [
  "identity",
  "policy",
  "observability",
];

/** The role each feature's providers fill (mirrors `PlatformFeature::role`). */
export const FEATURE_ROLE: Record<PlatformFeature, string> = {
  identity: "auth",
  policy: "policy",
  observability: "tracing",
};

/** How the editor names each feature, and what turning it on does. */
export const FEATURE_LABEL: Record<
  PlatformFeature,
  { title: string; help: string }
> = {
  identity: {
    title: "Identity",
    help: "Sign-in at the gateway for API and UI routes; services that can take the signed-in user do. Object stores and databases are never gated.",
  },
  policy: {
    title: "Policy",
    help: "Authorize each signed-in request with a policy engine. Turns on identity.",
  },
  observability: {
    title: "Observability",
    help: "A tracing backend the gateway and every capable service export their traces to.",
  },
};

/** Whether `selection` runs a gateway (omitted means it does). */
export const gatewayEnabled = (selection: Pick<Selection, "gateway">) =>
  selection.gateway?.enabled !== false;

/** The catalog's providers of `feature`, the default first. */
export function providersOf(
  catalog: CatalogDto,
  feature: PlatformFeature,
): ModuleDto[] {
  return catalog.modules.filter((m) => m.provider_of === FEATURE_ROLE[feature]);
}

/** The module id filling `feature` in `selection`, if it is on. */
export function featureProvider(
  catalog: CatalogDto,
  selection: Selection,
  feature: PlatformFeature,
): string | undefined {
  const choice = selection.platform?.[feature];
  if (choice === undefined) return undefined;
  return choice === true ? providersOf(catalog, feature)[0]?.id : choice;
}

/** Whether `feature` can be turned on in `selection`: identity and policy need
 *  the gateway. */
export const featureAvailable = (
  selection: Selection,
  feature: PlatformFeature,
) => feature === "observability" || gatewayEnabled(selection);

/**
 * `selection` with `feature` set to `choice`, or off when `choice` is
 * undefined. Policy needs identity, so turning policy on turns identity on and
 * turning identity off turns policy off.
 */
export function withFeature(
  selection: Selection,
  feature: PlatformFeature,
  choice: FeatureChoice | undefined,
): Selection {
  const platform = { ...selection.platform };
  if (choice === undefined) delete platform[feature];
  else platform[feature] = choice;
  if (feature === "policy" && choice !== undefined) platform.identity ??= true;
  if (feature === "identity" && choice === undefined) delete platform.policy;
  return { ...selection, platform };
}

/**
 * `selection` with the gateway on or off. Turning it off turns off identity
 * and policy, which need it; the other features are kept, so turning it back
 * on restores nothing that was lost except those two.
 */
export function withGateway(selection: Selection, on: boolean): Selection {
  const { gateway: _gateway, ...rest } = selection;
  if (on) return rest;
  const {
    identity: _identity,
    policy: _policy,
    ...platform
  } = selection.platform ?? {};
  return { ...rest, gateway: { enabled: false }, platform };
}

/**
 * `selection` with `module`'s integration with `feature` on or off. Only an
 * opt-out is stored: turning it back on removes the entry.
 */
export function withIntegration(
  selection: Selection,
  module: string,
  feature: PlatformFeature,
  on: boolean,
): Selection {
  const integrations = { ...selection.integrations };
  const entry = { ...integrations[module] };
  if (on) delete entry[feature];
  else entry[feature] = false;
  if (Object.keys(entry).length === 0) delete integrations[module];
  else integrations[module] = entry;
  return { ...selection, integrations };
}

/** `module`'s integrations in a plan, in feature order. */
export function integrationsOf(
  integrations: Integration[] | undefined,
  module: string,
): Integration[] {
  return (integrations ?? [])
    .filter((i) => i.module === module)
    .sort(
      (a, b) =>
        PLATFORM_FEATURES.indexOf(a.feature) -
        PLATFORM_FEATURES.indexOf(b.feature),
    );
}
