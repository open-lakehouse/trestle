import { afterEach, describe, expect, test } from "bun:test";
import type { Transport } from "@connectrpc/connect";
import {
  getHost,
  hostFetch,
  hostTransport,
  registerHost,
  ServiceUnavailableError,
} from "./index.js";
import { createFakeHost } from "./testing.js";

afterEach(() => registerHost(null));

/** A client that captures its fetch once, at construction, like openapi-fetch. */
function createClient(fetch: typeof globalThis.fetch) {
  return {
    async catalogs() {
      const res = await fetch("/api/catalogs");
      return res.json();
    },
  };
}

const uc = (name: string) => ({ "/api/catalogs": [{ name }] });

describe("hostFetch late binding", () => {
  test("a host registered before the client is constructed", async () => {
    registerHost(createFakeHost({ services: { "unity-catalog": uc("main") } }));
    const client = createClient(hostFetch("unity-catalog"));
    expect(await client.catalogs()).toEqual([{ name: "main" }]);
  });

  test("a host registered after the client is constructed", async () => {
    const client = createClient(hostFetch("unity-catalog"));
    registerHost(createFakeHost({ services: { "unity-catalog": uc("late") } }));
    expect(await client.catalogs()).toEqual([{ name: "late" }]);
  });

  test("a re-registered host takes over from the next call", async () => {
    const client = createClient(hostFetch("unity-catalog"));
    registerHost(createFakeHost({ services: { "unity-catalog": uc("a") } }));
    expect(await client.catalogs()).toEqual([{ name: "a" }]);
    registerHost(createFakeHost({ services: { "unity-catalog": uc("b") } }));
    expect(await client.catalogs()).toEqual([{ name: "b" }]);
  });

  test("a binding swapped by an environment switch takes over", async () => {
    const host = createFakeHost({ services: { "unity-catalog": uc("dev") } });
    registerHost(host);
    const client = createClient(hostFetch("unity-catalog"));
    expect(await client.catalogs()).toEqual([{ name: "dev" }]);
    host.setService("unity-catalog", uc("staging"));
    expect(await client.catalogs()).toEqual([{ name: "staging" }]);
  });

  test("no host rejects with ServiceUnavailableError", async () => {
    const client = createClient(hostFetch("unity-catalog"));
    const error = await client.catalogs().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ServiceUnavailableError);
    expect((error as ServiceUnavailableError).moduleId).toBe("unity-catalog");
    expect((error as Error).message).toContain("no host is registered");
  });

  test("an unbound module rejects, naming the module", async () => {
    registerHost(createFakeHost({ services: { mlflow: {} } }));
    const error = await hostFetch("unity-catalog")("/x").catch(
      (e: unknown) => e,
    );
    expect(error).toBeInstanceOf(ServiceUnavailableError);
    expect((error as Error).message).toContain("doesn't bind it");
  });

  test("the fallback answers when there is no binding", async () => {
    const fallback = (async () =>
      Response.json("standalone")) as unknown as typeof fetch;
    const fetch = hostFetch("unity-catalog", { fallback });
    expect(await (await fetch("/x")).json()).toBe("standalone");
    registerHost(createFakeHost({ services: { "unity-catalog": uc("host") } }));
    expect(await (await fetch("/api/catalogs")).json()).toEqual([
      { name: "host" },
    ]);
  });
});

/** A transport that records which instance served a call. */
function namedTransport(name: string, calls: string[]): Transport {
  return {
    unary: async () => {
      calls.push(`${name}:unary`);
      return {} as Awaited<ReturnType<Transport["unary"]>>;
    },
    stream: async () => {
      calls.push(`${name}:stream`);
      return {} as Awaited<ReturnType<Transport["stream"]>>;
    },
  };
}

type UnaryArgs = Parameters<Transport["unary"]>;
type StreamArgs = Parameters<Transport["stream"]>;
const unaryArgs = [] as unknown as UnaryArgs;
const streamArgs = [] as unknown as StreamArgs;

describe("hostTransport late binding", () => {
  test("registered before or after, and swapped, each call resolves anew", async () => {
    const calls: string[] = [];
    const transport = hostTransport("headwaters");
    registerHost(
      createFakeHost({
        services: { headwaters: { transport: namedTransport("a", calls) } },
      }),
    );
    await transport.unary(...unaryArgs);
    registerHost(
      createFakeHost({
        services: { headwaters: { transport: namedTransport("b", calls) } },
      }),
    );
    await transport.unary(...unaryArgs);
    await transport.stream(...streamArgs);
    expect(calls).toEqual(["a:unary", "b:unary", "b:stream"]);
  });

  test("a binding without a transport rejects", async () => {
    registerHost(createFakeHost({ services: { headwaters: {} } }));
    const error = await hostTransport("headwaters")
      .unary(...unaryArgs)
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ServiceUnavailableError);
    expect((error as Error).message).toContain("no ConnectRPC transport");
  });

  test("no host rejects rather than throwing synchronously", async () => {
    const pending = hostTransport("headwaters").stream(...streamArgs);
    expect(pending).toBeInstanceOf(Promise);
    expect(await pending.catch((e: unknown) => e)).toBeInstanceOf(
      ServiceUnavailableError,
    );
  });
});

test("registerHost(null) clears the host", () => {
  registerHost(createFakeHost());
  expect(getHost()).not.toBeNull();
  registerHost(null);
  expect(getHost()).toBeNull();
});
