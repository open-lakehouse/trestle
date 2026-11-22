import { describe, expect, test } from "bun:test";
import { TooltipProvider } from "@open-lakehouse/ui-kit";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  ContributionsProvider,
  type EditorContributions,
} from "./contributions.js";
import {
  createEditor,
  type EditorHandle,
  EditorProvider,
  type EditorStore,
  useEditor,
} from "./editorStore.js";
import { envSubTabs, INITIAL_VIEW, resolveSubTab, tabKey } from "./editors.js";
import { EnvironmentPane } from "./envPane.js";
import {
  createWorkbench,
  EDITOR_VIEW,
  WorkbenchProvider,
} from "./workbench.js";

const html = (node: ReactNode) =>
  renderToStaticMarkup(<TooltipProvider>{node}</TooltipProvider>);

/** Counts the times `store` tells its listeners about a change. */
function changes(store: EditorStore) {
  let calls = 0;
  store.subscribe(() => calls++);
  return () => calls;
}

const keys = (store: EditorStore) => store.state().tabs.map((t) => tabKey(t));

describe("editor store", () => {
  test("starts with no tab open, or with the ones it is given", () => {
    const empty = createEditor().state();
    expect(empty).toEqual({ tabs: [], activeKey: undefined, views: {} });

    // An active key that isn't open falls back to the last tab.
    const seeded = createEditor({
      tabs: [{ env: "a" }, { env: "b" }],
      activeKey: "nowhere",
    });
    expect(seeded.state().activeKey).toBe(tabKey({ env: "b" }));
  });

  test("openEnvironment opens a tab and activates it; reopening only activates", () => {
    const store = createEditor();
    const count = changes(store);
    store.openEnvironment("dev");
    store.openEnvironment("prod");
    expect(keys(store)).toEqual([
      tabKey({ env: "dev" }),
      tabKey({ env: "prod" }),
    ]);
    expect(store.state().activeKey).toBe(tabKey({ env: "prod" }));
    expect(count()).toBe(2);

    store.openEnvironment("dev");
    expect(keys(store)).toHaveLength(2);
    expect(store.state().activeKey).toBe(tabKey({ env: "dev" }));
    expect(count()).toBe(3);

    // Already open and active: nothing changed, so nobody is told.
    store.openEnvironment("dev");
    expect(count()).toBe(3);
  });

  test("a sub-tab is kept on the environment; disabled and unknown ones resolve as the pane does", () => {
    const store = createEditor();
    store.openEnvironment("dev", { subTab: "configure" });
    const view = store.state().views.dev;
    expect(view?.subTab).toBe("configure");

    const tabs = envSubTabs(
      [
        {
          id: "configure",
          label: "Configure",
          enabled: false,
          reason: "dev is off",
        },
      ],
      { revisions: false },
    );
    // A disabled tab is kept, so the pane can say why it is unavailable.
    expect(resolveSubTab(view ?? INITIAL_VIEW, tabs)).toBe("configure");
    // An unknown one falls back to the first enabled tab, the diagram.
    expect(resolveSubTab({ subTab: "nowhere" }, tabs)).toBe("diagram");
  });

  test("revisions and comparisons are tabs of their own", () => {
    const store = createEditor();
    store.openEnvironment("dev", { revision: 2 });
    store.openEnvironment("dev", { compare: { from: 1, to: "draft" } });
    store.openEnvironment("dev");
    expect(store.state().tabs).toHaveLength(3);
    expect(store.state().activeKey).toBe(tabKey({ env: "dev" }));
    expect(new Set(keys(store)).size).toBe(3);
  });

  test("activate ignores keys that aren't open", () => {
    const store = createEditor();
    store.openEnvironment("a");
    store.openEnvironment("b");
    const count = changes(store);
    store.activate("nowhere");
    expect(count()).toBe(0);
    expect(store.state().activeKey).toBe(tabKey({ env: "b" }));

    store.activate(tabKey({ env: "a" }));
    expect(store.state().activeKey).toBe(tabKey({ env: "a" }));
    expect(count()).toBe(1);
  });

  test("closing the active tab activates its right neighbour, else its left one", () => {
    const store = createEditor();
    for (const env of ["a", "b", "c"]) store.openEnvironment(env);
    store.activate(tabKey({ env: "b" }));

    // A tab that isn't active leaves the active one alone.
    store.close(tabKey({ env: "a" }));
    expect(store.state().activeKey).toBe(tabKey({ env: "b" }));

    // The active tab's right neighbour takes over.
    store.close(tabKey({ env: "b" }));
    expect(store.state().activeKey).toBe(tabKey({ env: "c" }));

    // With no right neighbour, the left one does.
    store.close(tabKey({ env: "c" }));
    expect(keys(store)).toEqual([]);
    expect(store.state().activeKey).toBeUndefined();
  });

  test("closing a key that isn't open changes nothing", () => {
    const store = createEditor();
    store.openEnvironment("a");
    const count = changes(store);
    store.close("nowhere");
    expect(count()).toBe(0);
    expect(keys(store)).toHaveLength(1);
  });

  test("setView is told only about a change of the environment's view", () => {
    const store = createEditor();
    const count = changes(store);
    // The diagram is the view an environment has without an entry.
    store.setView("dev", INITIAL_VIEW);
    expect(count()).toBe(0);

    store.setView("dev", { subTab: "files" });
    expect(count()).toBe(1);
    expect(store.state().views.dev).toEqual({ subTab: "files" });

    store.setView("dev", { subTab: "files" });
    expect(count()).toBe(1);

    store.setView("dev", { subTab: "files", filePath: "compose.yaml" });
    expect(count()).toBe(2);
  });

  test("syncEnvironments opens the first environment once, and only when nothing is open", () => {
    // No environment, no tab.
    const none = createEditor();
    none.syncEnvironments([]);
    expect(keys(none)).toEqual([]);

    const store = createEditor();
    store.syncEnvironments(["a", "b"]);
    expect(keys(store)).toEqual([tabKey({ env: "a" })]);
    expect(store.state().activeKey).toBe(tabKey({ env: "a" }));

    // Closing the tab doesn't bring it back: the start-up happened once.
    store.close(tabKey({ env: "a" }));
    store.syncEnvironments(["a", "b"]);
    expect(keys(store)).toEqual([]);
  });

  test("syncEnvironments keeps a tab a host opened before the shell started", () => {
    const store = createEditor();
    store.openEnvironment("b", { subTab: "configure" });
    store.syncEnvironments(["a", "b"]);
    expect(keys(store)).toEqual([tabKey({ env: "b" })]);
    expect(store.state().views.b?.subTab).toBe("configure");
  });

  test("syncEnvironments closes the tabs of environments that left, and settles on the last tab", () => {
    const store = createEditor();
    store.syncEnvironments(["a", "b"]);
    store.openEnvironment("b");
    store.syncEnvironments(["a"]);
    expect(keys(store)).toEqual([tabKey({ env: "a" })]);
    expect(store.state().activeKey).toBe(tabKey({ env: "a" }));

    const count = changes(store);
    store.syncEnvironments(["a"]);
    expect(count()).toBe(0);
  });

  test("subscribe's unsubscribe stops the calls", () => {
    const store = createEditor();
    let calls = 0;
    const unsubscribe = store.subscribe(() => calls++);
    store.openEnvironment("a");
    unsubscribe();
    store.openEnvironment("b");
    expect(calls).toBe(1);
  });
});

describe("editor provider", () => {
  test("shares a host's store, and useEditor reads its tabs", () => {
    const store = createEditor();
    store.openEnvironment("dev", { subTab: "configure" });
    function Probe() {
      const { active, views } = useEditor();
      return (
        <span>
          {active?.env}|{active && views[active.env]?.subTab}
        </span>
      );
    }
    expect(
      renderToStaticMarkup(
        <EditorProvider editor={store}>
          <Probe />
        </EditorProvider>,
      ),
    ).toContain("dev|configure");
  });

  test("useEditor needs a provider above it", () => {
    function Probe() {
      useEditor();
      return null;
    }
    expect(() => renderToStaticMarkup(<Probe />)).toThrow(
      "useEditor needs an <EditorProvider> above it",
    );
  });

  test("openEnvironment from useEditor brings the editor tabs over a workbench view", () => {
    const store = createEditor();
    const workbench = createWorkbench("apps");
    let handle: EditorHandle | undefined;
    function Probe() {
      handle = useEditor();
      return null;
    }
    renderToStaticMarkup(
      <WorkbenchProvider workbench={workbench}>
        <EditorProvider editor={store}>
          <Probe />
        </EditorProvider>
      </WorkbenchProvider>,
    );
    handle?.openEnvironment("dev", { subTab: "configure" });
    expect(workbench.active()).toBe(EDITOR_VIEW);
    expect(store.state().views.dev?.subTab).toBe("configure");
  });
});

describe("a host's store opens an environment's configure tab", () => {
  test("the shell's pane shows the host's sub-tab for the environment the store opened", () => {
    // The host opens env X on its Configure tab, before the shell mounts.
    const store = createEditor();
    store.openEnvironment("env-x", { subTab: "configure" });
    const contributions: EditorContributions = {
      environment: {
        tabs: (envId) => [
          {
            id: "configure",
            label: "Configure",
            enabled: true,
            render: () => <div id={`configure-${envId}`} />,
          },
        ],
      },
    };
    // The shell draws the active tab's pane from the store, as this does.
    function Shell() {
      const { active, views } = useEditor();
      if (!active) return null;
      return (
        <EnvironmentPane
          envId={active.env}
          name={active.env}
          view={views[active.env] ?? INITIAL_VIEW}
          onView={() => {}}
          revisions={{ revisions: false }}
          diagram={<div id="diagram" />}
          files={<div id="files" />}
          changes={<div id="changes" />}
        />
      );
    }
    const out = html(
      <EditorProvider editor={store}>
        <ContributionsProvider contributions={contributions}>
          <Shell />
        </ContributionsProvider>
      </EditorProvider>,
    );
    expect(out).toContain('<div id="configure-env-x"></div>');
    expect(out).not.toContain('id="diagram"');
    expect(out).toContain(">Configure<");
  });
});
