import { describe, expect, test } from "bun:test";
import { strFromU8, unzipSync } from "fflate";
import type { Bundle } from "../types";
import { buildBundleZip, fileMode, generateCredentials } from "./bundleZip";

const bundle: Bundle = {
  env_name: "lakehouse",
  files: [
    {
      path: "compose.yaml",
      contents: "services: {}\n",
      sensitive: false,
      executable: false,
    },
    {
      path: "modules/postgres/secrets/postgres_password",
      contents: "s3cr3t\n",
      sensitive: true,
      executable: false,
    },
    {
      path: "deploy/hosted-postgres-server.sh",
      contents: "#!/usr/bin/env bash\n",
      sensitive: false,
      executable: true,
    },
  ],
};

/** Read each entry's Unix mode back out of the zip's central directory. */
function centralModes(zip: Uint8Array): Map<string, number> {
  const view = new DataView(zip.buffer, zip.byteOffset, zip.byteLength);
  const modes = new Map<string, number>();
  for (let i = 0; i + 46 <= zip.length; i++) {
    if (view.getUint32(i, true) !== 0x02014b50) continue;
    const nameLen = view.getUint16(i + 28, true);
    const attrs = view.getUint32(i + 38, true);
    const name = strFromU8(zip.subarray(i + 46, i + 46 + nameLen));
    modes.set(name, (attrs >>> 16) & 0o777);
  }
  return modes;
}

describe("buildBundleZip", () => {
  test("puts every file under the environment's directory", () => {
    const files = unzipSync(buildBundleZip(bundle));
    expect(Object.keys(files).sort()).toEqual([
      "lakehouse/compose.yaml",
      "lakehouse/deploy/hosted-postgres-server.sh",
      "lakehouse/modules/postgres/secrets/postgres_password",
    ]);
    expect(
      strFromU8(files["lakehouse/modules/postgres/secrets/postgres_password"]),
    ).toBe("s3cr3t\n");
  });

  test("records owner-only secrets and executable scripts", () => {
    const modes = centralModes(buildBundleZip(bundle));
    expect(modes.get("lakehouse/compose.yaml")).toBe(0o644);
    expect(
      modes.get("lakehouse/modules/postgres/secrets/postgres_password"),
    ).toBe(0o600);
    expect(modes.get("lakehouse/deploy/hosted-postgres-server.sh")).toBe(0o755);
  });
});

describe("fileMode", () => {
  test("an executable secret stays owner-only", () => {
    expect(fileMode({ sensitive: true, executable: true })).toBe(0o700);
  });
});

describe("generateCredentials", () => {
  test("fills each credential within its bounds, freshly each time", () => {
    const specs = [
      { key: "access", description: "", min_len: 3, max_len: 20 },
      { key: "secret", description: "", min_len: 20, max_len: 128 },
    ];
    const first = generateCredentials(specs);
    expect(first.access).toMatch(/^[0-9a-f]{20}$/);
    expect(first.secret).toMatch(/^[0-9a-f]{64}$/);
    expect(generateCredentials(specs).secret).not.toBe(first.secret);
  });
});
