import { afterEach, describe, expect, test } from "bun:test";
import { renderToString } from "react-dom/server";
import { registerHost } from "./index.js";
import {
  HostProvider,
  useEnvironmentKey,
  useHost,
  useOptionalHost,
  useService,
} from "./react.js";
import { createFakeHost } from "./testing.js";

afterEach(() => registerHost(null));

function Probe({ id = "svc" }: { id?: string }) {
  const host = useHost();
  const binding = useService(id);
  const key = useEnvironmentKey();
  return (
    <span>{`${host.kind}|${binding?.placement ?? "unbound"}|${key}`}</span>
  );
}

describe("react bindings", () => {
  test("the provider supplies the host", () => {
    const host = createFakeHost({ services: { svc: {} } });
    const html = renderToString(
      <HostProvider host={host}>
        <Probe />
      </HostProvider>,
    );
    expect(html).toContain(`storybook|local|${host.environment.key}`);
  });

  test("the registered host is the fallback", () => {
    registerHost(createFakeHost({ kind: "desktop" }));
    expect(renderToString(<Probe />)).toContain("desktop|unbound|");
  });

  test("the provider wins over the registered host", () => {
    registerHost(createFakeHost({ kind: "desktop" }));
    const html = renderToString(
      <HostProvider host={createFakeHost({ kind: "web" })}>
        <Probe />
      </HostProvider>,
    );
    expect(html).toContain("web|");
  });

  test("useHost without any host is a wiring error", () => {
    expect(() => renderToString(<Probe />)).toThrow(/HostProvider/);
  });

  test("useOptionalHost without any host is null", () => {
    function Optional() {
      return <span>{useOptionalHost() === null ? "none" : "some"}</span>;
    }
    expect(renderToString(<Optional />)).toContain("none");
  });
});
