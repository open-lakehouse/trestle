// Package a rendered environment (`Planner.bundle`) as a zip download.
//
// The archive unpacks into one `<env_name>/` directory, ready for
// `docker compose up`. Unix modes ride along in the entries so `unzip` restores
// them: credential files owner-only, deploy scripts executable.

import { zipSync } from "fflate";
import type { Bundle, LocalCredentialDto } from "../types";

/** The longest generated value, even when a service accepts more. */
const MAX_GENERATED = 64;

/**
 * Fresh random values for the in-stack credentials: hex of the longest length
 * each service accepts (capped at {@link MAX_GENERATED} characters), from the
 * browser's CSPRNG.
 */
export function generateCredentials(
  specs: readonly LocalCredentialDto[],
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const spec of specs) {
    const bytes = new Uint8Array(
      Math.floor(Math.min(spec.max_len, MAX_GENERATED) / 2),
    );
    crypto.getRandomValues(bytes);
    out[spec.key] = Array.from(bytes, (b) =>
      b.toString(16).padStart(2, "0"),
    ).join("");
  }
  return out;
}

/** The Unix mode a bundle file is written with. */
export function fileMode(file: {
  sensitive: boolean;
  executable: boolean;
}): number {
  if (file.executable) return file.sensitive ? 0o700 : 0o755;
  return file.sensitive ? 0o600 : 0o644;
}

/** A zip entry's external attributes for a regular file with Unix `mode`. */
const S_IFREG = 0o100000;
const UNIX = 3;

type ZipEntries = Parameters<typeof zipSync>[0];

/** Add every file of `bundle` to `entries` under `root/`. */
function addBundle(entries: ZipEntries, root: string, bundle: Bundle): void {
  const encoder = new TextEncoder();
  for (const file of bundle.files) {
    entries[`${root}/${file.path}`] = [
      encoder.encode(file.contents),
      { os: UNIX, attrs: ((S_IFREG | fileMode(file)) << 16) >>> 0 },
    ];
  }
}

/** Zip every file of `bundle` under a `<env_name>/` root. */
export function buildBundleZip(bundle: Bundle): Uint8Array {
  const entries: ZipEntries = {};
  addBundle(entries, bundle.env_name, bundle);
  return zipSync(entries, { level: 6 });
}

/**
 * Zip a workspace the way `trestle workspace` lays it out: a `<name>/` root
 * holding `trestle-workspace.toml` (`manifest`) and each member's bundle in a
 * directory named after it.
 */
export function buildWorkspaceZip(
  name: string,
  manifest: string,
  members: readonly Bundle[],
): Uint8Array {
  const entries: ZipEntries = {
    [`${name}/trestle-workspace.toml`]: [
      new TextEncoder().encode(manifest),
      { os: UNIX, attrs: ((S_IFREG | 0o644) << 16) >>> 0 },
    ],
  };
  for (const bundle of members) {
    addBundle(entries, `${name}/${bundle.env_name}`, bundle);
  }
  return zipSync(entries, { level: 6 });
}
