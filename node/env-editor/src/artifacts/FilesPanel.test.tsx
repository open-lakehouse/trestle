import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { PlanResult } from "../types.js";
import { FilesPanel } from "./FilesPanel.js";

const plan = {
  files: [
    { path: "compose.yaml", contents: "services: {}" },
    { path: "modules/postgres/init.sql", contents: "select 1;" },
  ],
} as unknown as PlanResult;

const render = (selectedPath?: string) =>
  renderToStaticMarkup(
    <FilesPanel
      plan={plan}
      selectedPath={selectedPath}
      onSelect={() => {}}
      renderFile={(path, read) => (
        <output data-path={path}>{String(read(path))}</output>
      )}
    />,
  );

describe("FilesPanel", () => {
  test("driven from outside, it opens the given file through renderFile", () => {
    const out = render("modules/postgres/init.sql");
    expect(out).toContain(
      '<output data-path="modules/postgres/init.sql">select 1;</output>',
    );
  });

  test("a path the plan doesn't render opens its first file", () => {
    expect(render("gone.yaml")).toContain('data-path="compose.yaml"');
  });
});
