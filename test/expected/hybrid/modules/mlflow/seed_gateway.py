"""Seed MLflow's AI Gateway with one endpoint per configured model.

Runs once per `docker compose up` (the `mlflow-gateway-init` service) and is idempotent, so
endpoints and settings made in the MLflow UI survive re-runs.

MLFLOW_GATEWAY_MODELS lists `provider:model` pairs. Each becomes an endpoint *named the model
id*, so clients that ask for the model by its usual id (Claude Code, the OpenAI SDKs) reach it
unchanged. Every endpoint this job makes has usage tracking on: MLflow records each call through
it as a trace in the endpoint's own experiment, `gateway/<endpoint>`. Tracking is turned back on
if it was switched off. A `databricks:` pair names a workspace serving endpoint, and
its endpoint drops the `databricks-` prefix (`databricks:databricks-claude-opus-5-5` serves
`claude-opus-5-5`).

MLFLOW_UPSTREAM picks where each pair's calls go, its *route*:

- `keys`: the provider itself, with its key from this container's environment
  (ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY).
- `neon`: the Neon AI Gateway of the environment's branch, for `anthropic:` and `openai:` pairs
  (NEON_AI_GATEWAY_BASE_URL, with NEON_AI_GATEWAY_TOKEN as the key).
- `databricks:` pairs go to the workspace at DATABRICKS_HOST. With DATABRICKS_TOKEN set, that
  token is the key, under any upstream (it expires, so re-run with a fresh one). Without it,
  under the `databricks` upstream, the stored secret holds no token and the MLflow server's
  Databricks SDK authenticates on its own: with DATABRICKS_METADATA_SERVICE_URL, it fetches
  and refreshes tokens from that endpoint.

A pair whose credential is missing is skipped with a log line. Credentials only ever live here
and in MLflow (encrypted at rest), never in the clients.

The endpoints this job creates carry the route they were made for (the `trestle.route` tag).
When the route changes (a new upstream, a token set or unset), the job re-points the endpoint's
model definition and refreshes the secrets it routes through; an endpoint without the tag is
the user's and is left alone.

It then stays up, healthy, so `docker compose up --wait` returns once seeding is done instead
of failing on an exited container that no other service depends on.
"""

import json
import os
import pathlib
import signal
import sys
import time
import urllib.error
import urllib.request

READY = pathlib.Path("/tmp/seeded")
ROUTE_TAG = "trestle.route"
KEY_ENV = {
    "anthropic": "ANTHROPIC_API_KEY",
    "openai": "OPENAI_API_KEY",
    "gemini": "GEMINI_API_KEY",
}
# The Neon AI Gateway's surface per provider, under its bare base URL.
NEON_PATHS = {"anthropic": "/anthropic/v1", "openai": "/openai/v1"}
DATABRICKS_PREFIX = "databricks-"


def log(message):
    print(f"mlflow-gateway-init: {message}")


def parse_models(spec):
    pairs = []
    for item in spec.split(","):
        item = item.strip()
        if not item:
            continue
        provider, sep, model = item.partition(":")
        if not sep or not provider.strip() or not model.strip():
            sys.exit(f"mlflow-gateway-init: expected provider:model, got {item!r}")
        pairs.append((provider.strip(), model.strip()))
    return pairs


def endpoint_name(provider, model):
    """The endpoint a pair serves: the model id, without a serving endpoint's prefix."""
    if provider == "databricks" and model.startswith(DATABRICKS_PREFIX):
        return model[len(DATABRICKS_PREFIX) :]
    return model


def plan_route(provider, model, upstream, env):
    """Where a pair's calls go, or why it is skipped.

    Returns `(route, secret, reason)`. A route is the `trestle.route` tag value. A secret is a
    dict with the MLflow secret's `secret_name`, `provider`, `secret_value` and optional
    `auth_config`. Without a credential, `route` is None and `reason` says what is missing.
    """
    if provider == "databricks":
        host = env.get("DATABRICKS_HOST", "").rstrip("/")
        if not host:
            return None, None, "DATABRICKS_HOST is not set"
        token = env.get("DATABRICKS_TOKEN", "")
        if token:
            secret = {
                "secret_name": "databricks-token",
                "provider": "databricks",
                "secret_value": {"api_key": token},
                "auth_config": {"api_base": host},
            }
            return "databricks-token", secret, None
        if upstream != "databricks":
            # Only the databricks upstream gives the server the SDK's environment.
            return None, None, "DATABRICKS_TOKEN is not set (or choose the databricks upstream)"
        # No token: the server's Databricks SDK authenticates from its environment.
        secret = {
            "secret_name": "databricks",
            "provider": "databricks",
            "secret_value": {},
            "auth_config": {"api_base": host},
        }
        return "databricks", secret, None
    if upstream == "neon":
        path = NEON_PATHS.get(provider)
        if path is None:
            return None, None, f"the Neon AI Gateway upstream serves no {provider} provider"
        base = env.get("NEON_AI_GATEWAY_BASE_URL", "").rstrip("/")
        token = env.get("NEON_AI_GATEWAY_TOKEN", "")
        if not base or not token:
            return None, None, "NEON_AI_GATEWAY_BASE_URL or NEON_AI_GATEWAY_TOKEN is not set"
        secret = {
            "secret_name": f"neon-{provider}",
            "provider": provider,
            "secret_value": {"api_key": token},
            "auth_config": {"api_base": base + path},
        }
        return "neon", secret, None
    key_var = KEY_ENV.get(provider)
    if key_var is None:
        return None, None, f"no key variable known for {provider}"
    key = env.get(key_var, "")
    if not key:
        return None, None, f"{key_var} is not set"
    secret = {"secret_name": provider, "provider": provider, "secret_value": {"api_key": key}}
    return "keys", secret, None


class Gateway:
    """MLflow's gateway API."""

    def __init__(self, base):
        self.base = base.rstrip("/")
        self.api = self.base + "/api/3.0/mlflow/gateway"

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            self.api + path,
            data=data,
            method=method,
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw = resp.read()
        return json.loads(raw) if raw else {}

    def wait(self, deadline_s=120):
        start = time.monotonic()
        while True:
            try:
                urllib.request.urlopen(self.base + "/health", timeout=5).close()
                return
            except (urllib.error.URLError, OSError):
                if time.monotonic() - start > deadline_s:
                    sys.exit("mlflow-gateway-init: MLflow did not become healthy")
                time.sleep(2)


def ensure_secret(gw, secrets, refreshed, want):
    """The stored secret `want` names: created when missing, else updated once per run with
    the current value (a token may have been re-minted). A provider key (`keys`) is left as
    stored, so one changed in the UI survives."""
    name = want["secret_name"]
    have = secrets.get(name)
    if have is None:
        secrets[name] = gw.call("POST", "/secrets/create", want)["secret"]
        log(f"stored the {name} secret")
    elif name not in refreshed and want.get("auth_config") is not None:
        body = {"secret_id": have["secret_id"], "auth_config": want["auth_config"]}
        if want["secret_value"]:
            body["secret_value"] = want["secret_value"]
        gw.call("POST", "/secrets/update", body)
        log(f"refreshed the {name} secret")
    refreshed.add(name)
    return secrets[name]


def route_of(endpoint):
    """The route an endpoint was made for, or None for one this job didn't make."""
    for tag in endpoint.get("tags") or []:
        if tag.get("key") == ROUTE_TAG:
            return tag.get("value")
    # Made before the tag existed: the endpoint's own definition, bound to a provider key.
    for mapping in endpoint.get("model_mappings") or []:
        definition = mapping.get("model_definition") or {}
        if definition.get("name") == endpoint.get("name") and definition.get("secret_name") in KEY_ENV:
            return "keys"
    return None


def traced(endpoint):
    """Whether MLflow traces the endpoint's calls: with usage tracking on, each call becomes a
    trace in the endpoint's experiment (`gateway/<endpoint>`, made with the endpoint)."""
    return bool(endpoint.get("usage_tracking")) and bool(endpoint.get("experiment_id"))


def seed(gw, pairs, upstream, env):
    secrets = {s["secret_name"]: s for s in gw.call("GET", "/secrets/list").get("secrets", [])}
    definitions = {
        d["name"]: d for d in gw.call("GET", "/model-definitions/list").get("model_definitions", [])
    }
    endpoints = {e["name"]: e for e in gw.call("GET", "/endpoints/list").get("endpoints", [])}
    refreshed = set()

    for provider, model in pairs:
        name = endpoint_name(provider, model)
        existing = endpoints.get(name)
        if existing is not None and route_of(existing) is None:
            log(f"endpoint {name} exists (not made by this job); leaving it alone")
            continue
        route, want, reason = plan_route(provider, model, upstream, env)
        if route is None:
            log(f"skipping {name}: {reason}")
            continue
        secret = ensure_secret(gw, secrets, refreshed, want)
        definition = {
            "secret_id": secret["secret_id"],
            "provider": want["provider"],
            "model_name": model,
        }
        if name in definitions:
            have = definitions[name]
            if any(have.get(k) != v for k, v in definition.items()):
                gw.call(
                    "POST",
                    "/model-definitions/update",
                    {"model_definition_id": have["model_definition_id"], **definition},
                )
        else:
            definitions[name] = gw.call(
                "POST", "/model-definitions/create", {"name": name, **definition}
            )["model_definition"]
        if existing is None:
            existing = gw.call(
                "POST",
                "/endpoints/create",
                {
                    "name": name,
                    "model_configs": [
                        {
                            "model_definition_id": definitions[name]["model_definition_id"],
                            "linkage_type": "PRIMARY",
                            "weight": 1.0,
                        }
                    ],
                    "usage_tracking": True,
                },
            )["endpoint"]
            log(f"created endpoint {name} ({provider}, {route})")
        elif route_of(existing) != route:
            log(f"re-pointed endpoint {name} ({provider}, {route_of(existing)} -> {route})")
        else:
            log(f"endpoint {name} exists ({route})")
        if not traced(existing):
            existing = gw.call(
                "POST",
                "/endpoints/update",
                {"endpoint_id": existing["endpoint_id"], "usage_tracking": True},
            )["endpoint"]
            log(f"turned on tracing for endpoint {name}")
        gw.call(
            "POST",
            "/endpoints/set-tag",
            {"endpoint_id": existing["endpoint_id"], "key": ROUTE_TAG, "value": route},
        )


def main():
    pairs = parse_models(os.environ.get("MLFLOW_GATEWAY_MODELS", ""))
    if not pairs:
        log("no models configured; nothing to seed")
        return
    gw = Gateway(os.environ["MLFLOW_URL"])
    gw.wait()
    seed(gw, pairs, os.environ.get("MLFLOW_UPSTREAM", "keys"), os.environ)


if __name__ == "__main__":
    # Compose logs aren't a TTY, and the process never exits to flush them.
    sys.stdout.reconfigure(line_buffering=True)
    try:
        main()
    except urllib.error.HTTPError as err:
        sys.exit(f"mlflow-gateway-init: {err.code} {err.reason}: {err.read().decode(errors='replace')}")
    READY.touch()
    signal.pause()
