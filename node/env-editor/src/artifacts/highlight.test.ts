import { describe, expect, test } from "bun:test";
import { highlightLines, languageFor, plainLines } from "./highlight";

describe("languageFor", () => {
  test.each([
    ["modules/envoy/envoy.yaml", "yaml"],
    ["users.yml", "yaml"],
    ["config.toml", "toml"],
    ["modules/postgres/init-databases.sh", "shellscript"],
    ["modules/rustfs/sts_shim.py", "python"],
    ["secrets/server.properties", "properties"],
    ["client/host.env", "dotenv"],
    [".env", "dotenv"],
    ["templates/compose.yaml.jinja", "yaml"],
    ["Dockerfile", "dockerfile"],
    ["modules/authelia/README.md", "markdown"],
    ["policies/authz.rego", "plaintext"],
    ["METADATA", "plaintext"],
    [".gitignore", "plaintext"],
  ])("%s → %s", (path, lang) => {
    expect(languageFor(path)).toBe(lang as ReturnType<typeof languageFor>);
  });
});

describe("plainLines", () => {
  test("one uncolored token per non-empty line, no trailing empty line", () => {
    expect(plainLines("a\n\nb\n")).toEqual([
      [{ content: "a" }],
      [],
      [{ content: "b" }],
    ]);
  });
});

describe("highlightLines", () => {
  test("colors tokens for both themes and keeps the text", async () => {
    const code = "services:\n  db:\n    image: postgres:17\n";
    const lines = await highlightLines(code, "yaml");
    expect(lines).toHaveLength(3);
    expect(
      lines.map((line) => line.map((t) => t.content).join("")).join("\n"),
    ).toBe(code.trimEnd());
    const key = lines[0]?.[0];
    expect(key?.light).toMatch(/^#/);
    expect(key?.dark).toMatch(/^#/);
    expect(key?.light).not.toBe(key?.dark);
  });

  test("plaintext needs no highlighter", async () => {
    expect(await highlightLines("x\n", "plaintext")).toEqual([
      [{ content: "x" }],
    ]);
  });
});
