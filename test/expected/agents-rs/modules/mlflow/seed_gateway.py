"""Seed MLflow's AI Gateway with one endpoint per configured model.

Runs once per `docker compose up` (the `mlflow-gateway-init` service) and is idempotent:
anything that already exists is left alone, so endpoints, keys and settings changed in the
MLflow UI survive re-runs.

For each `provider:model` pair in MLFLOW_GATEWAY_MODELS whose provider key is set in this
container's environment (e.g. ANTHROPIC_API_KEY), it ensures:

- a gateway secret named after the provider, holding that key (encrypted at rest by MLflow);
- a model definition binding the secret to the model;
- an endpoint *named the model id*, with usage tracking on, so clients that ask for the model
  by its usual id (Claude Code, the OpenAI SDKs) reach it unchanged.

A pair whose key is missing is skipped with a log line; add the key and re-run, or create the
endpoint in the MLflow UI. The keys only ever live here and in MLflow, never in the clients.

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

BASE = os.environ["MLFLOW_URL"].rstrip("/")
API = BASE + "/api/3.0/mlflow/gateway"
READY = pathlib.Path("/tmp/seeded")
KEY_ENV = {
    "anthropic": "ANTHROPIC_API_KEY",
    "openai": "OPENAI_API_KEY",
    "gemini": "GEMINI_API_KEY",
}


def call(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        API + path,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        raw = resp.read()
    return json.loads(raw) if raw else {}


def wait_for_server(deadline_s=120):
    start = time.monotonic()
    while True:
        try:
            urllib.request.urlopen(BASE + "/health", timeout=5).close()
            return
        except (urllib.error.URLError, OSError):
            if time.monotonic() - start > deadline_s:
                sys.exit("mlflow-gateway-init: MLflow did not become healthy")
            time.sleep(2)


def parse_models(spec):
    pairs = []
    for item in spec.split(","):
        item = item.strip()
        if not item:
            continue
        provider, sep, model = item.partition(":")
        if not sep or not provider or not model:
            sys.exit(f"mlflow-gateway-init: expected provider:model, got {item!r}")
        pairs.append((provider.strip(), model.strip()))
    return pairs


def main():
    pairs = parse_models(os.environ.get("MLFLOW_GATEWAY_MODELS", ""))
    if not pairs:
        print("mlflow-gateway-init: no models configured; nothing to seed")
        return
    wait_for_server()

    secrets = {s["secret_name"]: s for s in call("GET", "/secrets/list").get("secrets", [])}
    definitions = {
        d["name"]: d for d in call("GET", "/model-definitions/list").get("model_definitions", [])
    }
    endpoints = {e["name"] for e in call("GET", "/endpoints/list").get("endpoints", [])}

    for provider, model in pairs:
        if model in endpoints:
            print(f"mlflow-gateway-init: endpoint {model} exists")
            continue
        key_var = KEY_ENV.get(provider)
        if key_var is None:
            print(f"mlflow-gateway-init: skipping {model}: no key variable known for {provider}")
            continue
        if provider not in secrets:
            key = os.environ.get(key_var, "")
            if not key:
                print(f"mlflow-gateway-init: skipping {model}: {key_var} is not set")
                continue
            secrets[provider] = call(
                "POST",
                "/secrets/create",
                {"secret_name": provider, "provider": provider, "secret_value": {"api_key": key}},
            )["secret"]
            print(f"mlflow-gateway-init: stored the {provider} key")
        if model not in definitions:
            definitions[model] = call(
                "POST",
                "/model-definitions/create",
                {
                    "name": model,
                    "secret_id": secrets[provider]["secret_id"],
                    "provider": provider,
                    "model_name": model,
                },
            )["model_definition"]
        call(
            "POST",
            "/endpoints/create",
            {
                "name": model,
                "model_configs": [
                    {
                        "model_definition_id": definitions[model]["model_definition_id"],
                        "linkage_type": "PRIMARY",
                        "weight": 1.0,
                    }
                ],
                "usage_tracking": True,
            },
        )
        endpoints.add(model)
        print(f"mlflow-gateway-init: created endpoint {model} ({provider})")


if __name__ == "__main__":
    # Compose logs aren't a TTY, and the process never exits to flush them.
    sys.stdout.reconfigure(line_buffering=True)
    try:
        main()
    except urllib.error.HTTPError as err:
        sys.exit(f"mlflow-gateway-init: {err.code} {err.reason}: {err.read().decode(errors='replace')}")
    READY.touch()
    signal.pause()
