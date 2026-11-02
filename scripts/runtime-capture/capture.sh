#!/usr/bin/env bash
# Capture what the Docker CLI prints for the commands olai-stack-runtime parses, as test
# fixtures: `compose ls/ps --format json`, `docker events`, `compose logs` and the
# `up -d --wait` outcome, healthy and unhealthy. See docs/runtime.md.
#
#   scripts/runtime-capture/capture.sh <docker-context> [<engine-name>]
#
# Writes crates/stack-runtime/tests/fixtures/<engine-name>/ (the engine name defaults to the
# context). Needs network access to pull python:3.13-alpine once.
set -euo pipefail

context=${1:?usage: capture.sh <docker-context> [<engine-name>]}
engine=${2:-$context}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out="$root/crates/stack-runtime/tests/fixtures/$engine"
project=trestle-capture
mkdir -p "$out"

docker_() { docker --context "$context" "$@"; }
compose() { docker_ compose --project-directory "$here" -f "$here/compose.yaml" "$@"; }

cleanup() {
  compose --profile bad down --volumes --remove-orphans >/dev/null 2>&1 || true
  [[ -n ${events_pid:-} ]] && kill "$events_pid" 2>/dev/null || true
}
trap cleanup EXIT
cleanup

{
  docker_ version --format '{{json .}}'
} >"$out/version.json"

# Events for the whole run, filtered to the project the way the runtime filters them.
docker_ events --format '{{json .}}' \
  --filter "label=com.docker.compose.project=$project" >"$out/events.jsonl" &
events_pid=$!
sleep 1

# A healthy start.
set +e
compose up -d --wait >"$out/up-wait-healthy.stdout" 2>"$out/up-wait-healthy.stderr"
echo $? >"$out/up-wait-healthy.exit"
set -e

compose ps --all --format json >"$out/ps-running.json"
docker_ compose ls --all --format json >"$out/ls-running.json"
sleep 3
compose logs --timestamps --no-color ticker >"$out/logs-ticker.txt"

# An unhealthy start: `broken` never passes its healthcheck.
set +e
compose --profile bad up -d --wait --wait-timeout 20 \
  >"$out/up-wait-unhealthy.stdout" 2>"$out/up-wait-unhealthy.stderr"
echo $? >"$out/up-wait-unhealthy.exit"
set -e
compose --profile bad ps --all --format json >"$out/ps-unhealthy.json"

# One service stopped, the others still up.
compose stop ticker >/dev/null 2>&1
compose --profile bad ps --all --format json >"$out/ps-partial.json"
docker_ compose ls --all --format json >"$out/ls-partial.json"

compose --profile bad down --volumes --remove-orphans >/dev/null 2>&1
docker_ compose ls --all --format json >"$out/ls-down.json"
sleep 1
kill "$events_pid" 2>/dev/null || true
events_pid=

# Other projects on the host are not ours to commit: keep only the capture project's rows.
python3 - "$out" "$project" <<'EOF'
import json, pathlib, sys
out, project = pathlib.Path(sys.argv[1]), sys.argv[2]
for path in out.glob("ls-*.json"):
    rows = json.loads(path.read_text() or "[]")
    rows = [r for r in rows if r.get("Name") == project]
    for r in rows:
        r["ConfigFiles"] = "/capture/compose.yaml"
    path.write_text(json.dumps(rows, indent=2) + "\n")
# Healthchecks run as execs: keep a few of those events, not one per probe.
events, seen = [], {}
for line in (out / "events.jsonl").read_text().splitlines():
    action = json.loads(line)["Action"].split(":")[0]
    if action.startswith("exec_"):
        seen[action] = seen.get(action, 0) + 1
        if seen[action] > 3:
            continue
    events.append(line)
(out / "events.jsonl").write_text("\n".join(events) + "\n")
EOF
# Scrub host paths from the rest.
for f in "$out"/*; do
  sed -i.bak -e "s#$here#/capture#g" -e "s#$HOME#/home#g" "$f" && rm -f "$f.bak"
done
echo "captured $engine fixtures in $out"
