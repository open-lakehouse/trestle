import { describe, expect, test } from "bun:test";
import type { CatalogDto, Integration, Selection } from "../types.js";
import {
  featureAvailable,
  featureProvider,
  integrationsOf,
  withFeature,
  withGateway,
  withIntegration,
} from "./platform.js";

const selection = (s: Partial<Selection>): Selection => ({
  modules: [],
  capabilities: [],
  knob_overrides: {},
  ...s,
});

const catalog = {
  modules: [
    { id: "authelia", provider_of: "auth" },
    { id: "jaeger", provider_of: "tracing" },
    { id: "other-tracer", provider_of: "tracing" },
  ],
} as unknown as CatalogDto;

describe("platform features", () => {
  test("policy turns identity on, and identity off turns policy off", () => {
    const on = withFeature(selection({}), "policy", true);
    expect(on.platform).toEqual({ policy: true, identity: true });
    expect(withFeature(on, "identity", undefined).platform).toEqual({});
  });

  test("turning the gateway off drops identity and policy only", () => {
    const s = selection({
      platform: { identity: true, policy: true, observability: true },
    });
    const off = withGateway(s, false);
    expect(off.gateway).toEqual({ enabled: false });
    expect(off.platform).toEqual({ observability: true });
    expect(featureAvailable(off, "identity")).toBe(false);
    expect(featureAvailable(off, "observability")).toBe(true);
    expect(withGateway(off, true).gateway).toBeUndefined();
  });

  test("a default choice resolves to the catalog's first provider", () => {
    expect(
      featureProvider(
        catalog,
        selection({ platform: { observability: true } }),
        "observability",
      ),
    ).toBe("jaeger");
    expect(
      featureProvider(
        catalog,
        selection({ platform: { observability: "other-tracer" } }),
        "observability",
      ),
    ).toBe("other-tracer");
    expect(featureProvider(catalog, selection({}), "identity")).toBeUndefined();
  });

  test("only integration opt-outs are stored", () => {
    const out = withIntegration(selection({}), "opa", "observability", false);
    expect(out.integrations).toEqual({ opa: { observability: false } });
    expect(
      withIntegration(out, "opa", "observability", true).integrations,
    ).toEqual({});
  });

  test("a module's integrations are listed in feature order", () => {
    const integrations: Integration[] = [
      {
        module: "envoy",
        feature: "observability",
        provider: "jaeger",
        state: "disabled",
      },
      {
        module: "envoy",
        feature: "identity",
        provider: "authelia",
        state: "active",
        protocol: "forward-auth",
      },
      {
        module: "opa",
        feature: "observability",
        provider: "jaeger",
        state: "disabled",
      },
    ];
    expect(integrationsOf(integrations, "envoy").map((i) => i.feature)).toEqual(
      ["identity", "observability"],
    );
  });
});
