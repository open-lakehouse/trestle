import { describe, expect, test } from "bun:test";
import { createFakeHost } from "./testing.js";

describe("createFakeHost", () => {
  test("route tables answer by path, or by method and path", async () => {
    const host = createFakeHost({
      services: {
        svc: { "/a": 1, "POST /b": { ok: true } },
      },
    });
    const { fetch } = host.services.get("svc") ?? {};
    expect(await (await fetch?.("/a"))?.json()).toBe(1);
    expect(await (await fetch?.("/b", { method: "POST" }))?.json()).toEqual({
      ok: true,
    });
    expect((await fetch?.("/b"))?.status).toBe(404);
  });

  test("handlers see the request, and requests are recorded", async () => {
    const host = createFakeHost({
      services: {
        svc: async (req) => new Response(await req.text()),
      },
    });
    const res = await host.services.get("svc")?.fetch("/echo", {
      method: "POST",
      body: "hi",
    });
    expect(await res?.text()).toBe("hi");
    expect(host.requests.map((r) => r.request.url)).toEqual([
      "http://fake.host/echo",
    ]);
    expect(host.requests[0]?.moduleId).toBe("svc");
  });

  test("a binding object keeps its fields and defaults the rest", () => {
    const host = createFakeHost({
      services: { ui: { embedUrl: "http://x/", placement: "remote" } },
    });
    const binding = host.services.get("ui");
    expect(binding?.embedUrl).toBe("http://x/");
    expect(binding?.placement).toBe("remote");
    expect(typeof binding?.fetch).toBe("function");
  });

  test("switching environments changes the key and notifies", () => {
    const host = createFakeHost();
    const seen: string[] = [];
    const unsubscribe = host.environment.subscribe(() =>
      seen.push(host.environment.key),
    );
    const first = host.environment.key;
    host.setEnvironment({ id: "staging", name: "staging" });
    expect(host.environment.current()?.id).toBe("staging");
    expect(host.environment.key).not.toBe(first);
    unsubscribe();
    host.setEnvironment({ id: "staging", name: "staging" });
    expect(seen).toHaveLength(1);
  });

  test("re-selecting the same environment still changes the key", () => {
    const host = createFakeHost();
    const before = host.environment.key;
    host.setEnvironment(host.environment.current());
    expect(host.environment.key).not.toBe(before);
  });

  test("setService adds and removes bindings", () => {
    const host = createFakeHost();
    expect(host.services.list?.()).toEqual([]);
    host.setService("svc", {});
    expect(host.services.list?.()).toEqual(["svc"]);
    host.setService("svc", undefined);
    expect(host.services.get("svc")).toBeUndefined();
  });

  test("no environment", () => {
    const host = createFakeHost({ environment: null });
    expect(host.environment.current()).toBeNull();
    expect(host.environment.key).toStartWith("none");
  });
});
