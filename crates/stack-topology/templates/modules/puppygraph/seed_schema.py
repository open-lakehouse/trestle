"""Upload the stack's graph schema to PuppyGraph.

Runs once per `docker compose up` (the `puppygraph-schema` service). It reads the
operator-owned graph schema (`graphs/puppygraph/schema.json`: which tables are nodes and
edges), merges in the generated catalog connection (`catalog.json`: the Unity Catalog
metastore, regenerated on every render), and POSTs the result to
PuppyGraph's `/schema` endpoint, which replaces the running schema.

A generated catalog replaces a same-named one in `schema.json`; any other catalogs there are
kept. While the schema maps no nodes, nothing is uploaded, so a schema built in PuppyGraph's
web UI is left alone until you write one here.
"""

import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request

SCHEMA_PATH = "/schema.json"
CATALOG_PATH = "/catalog.json"


def load(path):
    with open(path) as f:
        return json.load(f)


def merge(schema, generated):
    names = {c["name"] for c in generated}
    kept = [c for c in schema.get("catalog", []) if c.get("name") not in names]
    return {**schema, "catalog": generated + kept}


def upload(url, auth, body, deadline_s=120):
    data = json.dumps(body).encode()
    deadline = time.monotonic() + deadline_s
    while True:
        req = urllib.request.Request(
            url,
            data=data,
            method="POST",
            headers={"Content-Type": "application/json", "Authorization": auth},
        )
        try:
            # PuppyGraph validates every mapped table against the catalog before it answers.
            with urllib.request.urlopen(req, timeout=600) as resp:
                return resp.read().decode()
        except urllib.error.HTTPError as e:
            # The server is up and rejected the schema: retrying won't help.
            raise SystemExit(f"PuppyGraph rejected the schema ({e.code}): {e.read().decode()}")
        except (urllib.error.URLError, OSError) as e:
            if time.monotonic() > deadline:
                raise SystemExit(f"PuppyGraph is unreachable at {url}: {e}")
            time.sleep(3)


def main():
    schema = load(SCHEMA_PATH)
    if not schema.get("node"):
        print(
            "graphs/puppygraph/schema.json maps no nodes yet; skipping the upload. "
            "Add node/edge mappings over your Unity Catalog tables (catalog \"uc\") and "
            "re-run `docker compose up puppygraph-schema`."
        )
        return
    body = merge(schema, load(CATALOG_PATH))
    user = os.environ["PUPPYGRAPH_USERNAME"]
    password = os.environ["PUPPYGRAPH_PASSWORD"]
    auth = "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode()
    print(upload(os.environ["PUPPYGRAPH_SCHEMA_URL"], auth, body))
    print(
        f"Uploaded {len(body['node'])} node and {len(body.get('edge', []))} edge mapping(s)."
    )


if __name__ == "__main__":
    sys.exit(main())
