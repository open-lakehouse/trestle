import { describe, expect, test } from "bun:test";
import {
  fileSourceOf,
  filesByModule,
  moduleFiles,
  relativePath,
} from "./fileSource";

const paths = [
  "compose.yaml",
  "users.yml",
  "modules/postgres/init-databases.sh",
  "modules/postgres/compose.yaml",
  "modules/envoy/envoy.yaml",
  "modules/envoy/compose.yaml",
  "modules/envoy/certs.sh",
  "modules/postgres-extra/compose.yaml",
];

describe("moduleFiles", () => {
  test("lists a module's own files, compose.yaml first, then by path", () => {
    expect(moduleFiles(paths, "envoy")).toEqual([
      "modules/envoy/compose.yaml",
      "modules/envoy/certs.sh",
      "modules/envoy/envoy.yaml",
    ]);
  });

  test("doesn't match a module whose id only shares a prefix", () => {
    expect(moduleFiles(paths, "postgres")).toEqual([
      "modules/postgres/compose.yaml",
      "modules/postgres/init-databases.sh",
    ]);
  });

  test("a module without files has none", () => {
    expect(moduleFiles(paths, "jaeger")).toEqual([]);
  });
});

describe("filesByModule", () => {
  test("groups module files and leaves root files out", () => {
    const grouped = filesByModule(paths);
    expect([...grouped.keys()].sort()).toEqual([
      "envoy",
      "postgres",
      "postgres-extra",
    ]);
    expect(grouped.get("postgres-extra")).toEqual([
      "modules/postgres-extra/compose.yaml",
    ]);
  });
});

describe("relativePath", () => {
  test("strips the module directory", () => {
    expect(relativePath("modules/envoy/certs.sh", "envoy")).toBe("certs.sh");
    expect(relativePath("compose.yaml", "envoy")).toBe("compose.yaml");
  });
});

describe("fileSourceOf", () => {
  test("reads in-memory files and rejects unknown paths", () => {
    const source = fileSourceOf([{ path: "a.yaml", contents: "a: 1\n" }]);
    expect(source.paths).toEqual(["a.yaml"]);
    expect(source.read("a.yaml")).toBe("a: 1\n");
    expect(() => source.read("b.yaml")).toThrow();
  });
});
