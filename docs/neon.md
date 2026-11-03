# Neon from trestle: what the CLI does

Status: spike for WP2 (open-lakehouse/trestle#272), run on 2026-11-02 against
`neon` 8.16.0 (Homebrew), in a throwaway project in the open-lakehouse org. The
captured output is in `crates/stack-runtime/tests/fixtures/neon/8.16.0/`.
`scripts/neon-capture/capture.sh` re-captures it, and `scrub.py` strips secrets
and personal data. The `NeonControl` trait (#274) and its `FakeNeon` are built on
these fixtures.

## Verbs and their output

| What | Command | Output with `--output json` |
|---|---|---|
| Who am I | `neon me` | JSON object |
| Orgs | `neon orgs list` | JSON array |
| Projects | `neon projects list --org-id <org>` | JSON. **Without `--org-id` it prompts for an org, even with `--output json`**, so a non-interactive caller hangs. |
| Branches | `neon branches list/get/create/reset/delete --project-id <p>` | JSON. `create` and `reset` help output is broken in 8.16.0 (it prints the top-level help); their flags come from the source. |
| Create a branch | `branches create --name <n> --parent <branch> --expires-at <RFC 3339> [--no-secrets] [--annotation <json>]` | `{branch, endpoints, connection_uris}`. The connection URIs, with passwords, are included unless `--no-secrets`. |
| Reset | `branches reset <b> --parent [--preserve-under-name <n>]` | the branch, with `last_reset_at` |
| Expiry | `branches create --expires-at`, or `branches set-expiration <b> --expires-at` | `expires_at` and `ttl_interval_seconds` on the branch |
| Connection string | `neon connection-string <b> [--pooled]` | **a bare URL, not JSON** |
| Connection details | `neon connection-string <b> --extended` | JSON: `connection_string`, `host`, `role`, `password`, `database`, `options` |
| Functions | `neon functions list/get --branch <b>` | JSON, including `invocation_url` |
| Deploy a function | `neon functions deploy <slug> --src <dir> --branch <b>` | **text lines around a JSON object** |
| Buckets | `neon buckets list --branch <b>` | JSON array |
| Create a bucket | `neon buckets create <name> --branch <b>` | **a sentence, not JSON** |
| Storage credential | `neon credentials create --branch <b> --scope storage:read --scope storage:write` | **JSON followed by a `WARNING:` line**, with `token_id`, `api_token` and `s3_secret_access_key` |
| Storage endpoint | `neon api /projects/<p>/branches/<b>/storage` | `{enabled, s3_endpoint, region, force_path_style}` (no CLI verb) |
| Any API route | `neon api <path>` | the API's JSON |

So `NeonCli` can't parse every verb as one JSON document. It must take the first
JSON value of the output (`serde_json::Deserializer::into_iter`) and treat the
three text-only verbs (bare connection string, bucket create, and the lines
around deploy) as text.

## Branches

- **Ownership marker.** The hidden `--annotation '{"trestle.owner":"trestle",…}'`
  on `branches create` is stored. `branches get` and `list` don't show it, but the
  API does: `GET /projects/<p>/branches` returns an `annotations` map keyed by
  branch id. `NeonControl` lists branches through `neon api`, so it only resets
  or deletes branches it owns.
- **Expiry** is native (`expires_at`), and trestle-created branches set it.
- **Reset to parent** restores the data, nothing else (see functions).

## Postgres

- The direct and pooled hosts differ only by `-pooler`
  (`ep-…-pooler.c-8.<region>.aws.neon.tech`). `DATABASE_URL` is pooled and
  `DATABASE_URL_UNPOOLED` is direct.
- The URLs carry `sslmode=require&channel_binding=require`. libpq-based clients
  (psycopg, MLflow) accept that. JDBC clients (the Unity Catalog server) need
  checking in #281.
- `--extended` gives the parts for `PG*` (`PGHOST`, `PGUSER`, `PGPASSWORD`,
  `PGDATABASE`).

## Object storage

- A branch's storage is S3-compatible at
  `https://<branch-id>.storage.c-8.<region>.aws.neon.tech`, with **path-style
  addressing** (`force_path_style: true`) and region `eu-central-1`. The
  endpoint is only available through the API route above.
- **The credential's `token_id` is the S3 access key id**, and its secret is
  `s3_secret_access_key`. This is what `olai-vended-store/src/neon` already
  assumes. Verified with an `aws s3 cp` / `ls` round trip.
- The first bucket on a branch also creates a "Default object storage
  credential".
- **Children inherit everything.** A branch created from main gets main's
  buckets, their objects, and main's credentials, with the same token ids and
  `branch_id` still pointing at main. A credential minted on main works against
  the child's endpoint. So per-instance vending must mint credentials *on the
  child*, and revoke them on unbind. A credential minted on a parent leaks into
  every later branch.

## Functions

- Slugs are `[a-z0-9]{1,20}`, so they can't contain hyphens. Module ids such as
  `unity-catalog` need a slug mapping (`unitycatalog`) in #281.
- `invocation_url` is `https://<branch-id>-<slug>.compute.c-8.<region>.aws.neon.tech/`.
- **A new branch copies its parent's functions when it is created.** It gets its
  own deployment and its own URL. Branches created before a deploy don't get the
  function, and resetting them to the parent doesn't add it either. So:
  - ad hoc branches from the app inherit the parent's functions, and no
    `neon deploy` is needed when the parent already has them (this answers the
    open item in luminescent-lakehouse#14);
  - a long-lived branch needs a redeploy after the parent's functions change.
    The bind check (#274: "a missing function is an error at bind time") catches
    a missing one.
- **A function URL answered an anonymous request** (HTTP 200). Functions are
  public by default, so UC and lineage must check the caller's Neon Auth token
  themselves. A `functions:invoke` credential scope exists, but nothing enforces
  it on the default URL.

## What a branch supplies to an environment

The planner (#281) reads every Neon coordinate from a variable:

| Variable | From |
|---|---|
| `PGHOST`, `PGUSER`, `PGPASSWORD` | `connection-string --extended` (for `hosted-postgres` with vendor `neon`) |
| `NEON_S3_ENDPOINT`, `NEON_S3_REGION` | `neon api /projects/<p>/branches/<b>/storage` |
| `NEON_S3_ACCESS_KEY_ID`, `NEON_S3_SECRET_ACCESS_KEY` | a credential minted on the branch (`token_id`, `s3_secret_access_key`) |
| `<SLUG>_FUNCTION_URL` | `functions list` on the branch (`invocation_url` of the function with that slug) |

`Plan::neon_requirements()` lists the databases, buckets and functions (by
slug) one plan needs, plus these variables, so binding a branch can check it
before anything starts. `unity-catalog` deploys as `unitycatalog`, `headwaters`
as `headwaters`.

## Running `neon` from the desktop app

- Homebrew links `neon` and `neonctl` into `/opt/homebrew/bin` (Apple silicon)
  or `/usr/local/bin` (Intel). A Finder-launched app inherits launchd's PATH
  (`/usr/bin:/bin:/usr/sbin:/sbin`), which has neither.
- The `neon` script runs Node by absolute path
  (`#!/opt/homebrew/opt/node/bin/node`), so the app only needs to find `neon`
  itself, not Node.
- The app should therefore look for an explicit setting first, then
  `/opt/homebrew/bin`, `/usr/local/bin`, `~/.npm-global/bin` and the user's login
  shell (`$SHELL -lc 'command -v neon'`), and fall back to `neonctl`.
  `NeonCli::discover` does this in #274.
- Authentication lives in `~/.config/neon` (`--config-dir`) and is shared with the
  terminal. `NEON_API_KEY` or `--api-key` work without a browser login, for CI.
