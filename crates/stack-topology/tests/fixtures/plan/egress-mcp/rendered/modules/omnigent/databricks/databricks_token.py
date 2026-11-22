"""Print a Databricks workspace token, for the Omnigent host's `databricks` upstream.

The harness runs this as its key helper (Claude Code's `apiKeyHelper`) and runs it again when
the token goes stale. With DATABRICKS_METADATA_SERVICE_URL set, it asks that Databricks SDK
metadata-service endpoint (the desktop serves one per Databricks profile) for a fresh token;
otherwise it prints DATABRICKS_TOKEN, a static token that stops working when it expires.

The harness may run it without the host's environment, so each variable falls back to the
host process's (PID 1). Stdlib only: the host image has no Databricks SDK or CLI.
"""

import json
import os
import sys
import urllib.request


def host_env():
    try:
        with open("/proc/1/environ", "rb") as f:
            pairs = f.read().split(b"\0")
    except OSError:
        return {}
    out = {}
    for pair in pairs:
        key, sep, value = pair.partition(b"=")
        if sep:
            out[key.decode(errors="replace")] = value.decode(errors="replace")
    return out


def main():
    fallback = host_env()

    def var(name):
        return os.environ.get(name) or fallback.get(name, "")

    url, host = var("DATABRICKS_METADATA_SERVICE_URL"), var("DATABRICKS_HOST").rstrip("/")
    if url:
        req = urllib.request.Request(
            url,
            headers={"X-Databricks-Metadata-Version": "1", "X-Databricks-Host": host},
        )
        with urllib.request.urlopen(req, timeout=15) as resp:
            token = json.load(resp).get("access_token", "")
    else:
        token = var("DATABRICKS_TOKEN")
    if not token:
        sys.exit("databricks_token: set DATABRICKS_METADATA_SERVICE_URL or DATABRICKS_TOKEN")
    print(token)


if __name__ == "__main__":
    try:
        main()
    except OSError as err:
        sys.exit(f"databricks_token: {err}")
