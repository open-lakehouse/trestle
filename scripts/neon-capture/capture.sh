#!/usr/bin/env bash
# Capture what the Neon CLI prints for the verbs olai-stack-runtime's `NeonControl` drives, as
# test fixtures (#272 spike). Run against a THROWAWAY project: it creates, resets and deletes
# branches, a bucket, a credential and a function.
#
#   scripts/neon-capture/capture.sh <project-id> <org-id>
#
# Raw output goes to a private temp dir (it holds passwords and keys), then scrub.py writes the
# committable copy to crates/stack-runtime/tests/fixtures/neon/<neon version>/. Needs `neon` and
# `docker` (for the S3 round trip, run with the amazon/aws-cli image).
set -euo pipefail

P=${1:?usage: capture.sh <project-id> <org-id>}
ORG=${2:?usage: capture.sh <project-id> <org-id>}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
version=$(neon --version | tail -1)
raw=$(mktemp -d)
chmod 700 "$raw"
out="$root/crates/stack-runtime/tests/fixtures/neon/$version"
fn=$(mktemp -d)
trap 'rm -rf "$raw" "$fn"' EXIT

n() { neon "$@" --project-id "$P" --output json; }
expires() { python3 -c "import datetime;print((datetime.datetime.now(datetime.UTC)+datetime.timedelta(hours=3)).strftime('%Y-%m-%dT%H:%M:%SZ'))"; }
cd "$raw"

# Account and projects. Without --org-id, `projects list` stops at an interactive org picker.
neon me --output json >me.json
neon orgs list --output json >orgs-list.json
neon projects list --org-id "$ORG" --output json >projects-list.json

# Branches: create with a parent, an expiry and an annotation (the ownership marker), get,
# list (the CLI drops annotations; the API keeps them), connection strings, reset, delete.
n branches list >branches-list.json
n branches create --name spike-a --parent main --expires-at "$(expires)" \
  --annotation '{"trestle.owner":"trestle","trestle.instance":"spike"}' >branches-create.json
spike_a=$(python3 -c "import json;print(json.load(open('branches-create.json'))['branch']['id'])")
n branches get spike-a >branches-get.json
neon api "/projects/$P/branches/$spike_a" --output json >api-branch.json
neon api "/projects/$P/branches?search=spike" --output json >api-branches.json
neon connection-string spike-a --project-id "$P" --output json >cs-direct.json
neon connection-string spike-a --project-id "$P" --pooled --output json >cs-pooled.json
neon connection-string spike-a --project-id "$P" --extended --output json >cs-extended.json
n branches reset spike-a --parent >branches-reset.json

# Object storage: a bucket, a scoped credential (its token id is the S3 access key id) and the
# branch's endpoint (API only), then an S3 round trip with them.
main=$(python3 -c "import json;print([b['id'] for b in json.load(open('branches-list.json')) if b.get('default')][0])")
n functions list --branch main >functions-list-empty.json
n buckets list --branch main >buckets-list-empty.json
n credentials list --branch main >credentials-list-empty.json
n buckets create mlflow-artifacts --branch main >buckets-create.json
n buckets list --branch main >buckets-list.json
n credentials create --branch main --name trestle-spike-s3 \
  --scope storage:read --scope storage:write >credentials-create.json
neon api "/projects/$P/branches/$main/storage" --output json >api-storage.json

# Functions: deploy one to main, then see which branches get it.
(cd "$fn" && neon functions new basic --slug spikefn --yes --dir ./spikefn --project-id "$P" >/dev/null)
n functions templates list >functions-templates.json
(cd "$fn" && neon functions deploy spikefn --src ./spikefn --project-id "$P" --branch main \
  --output json) >functions-deploy.out
n functions list --branch main >functions-list.json
n functions get spikefn --branch main >functions-get.json
n branches create --name spike-b --parent main --expires-at "$(expires)" --no-secrets >branches-create-b.json
sleep 5
n functions list --branch spike-b >functions-list-child.json
n functions list --branch spike-a >functions-list-older.json
n branches reset spike-a --parent >branches-reset-2.json
n functions list --branch spike-a >functions-list-after-reset.json
spike_b=$(python3 -c "import json;print(json.load(open('branches-create-b.json'))['branch']['id'])")
n buckets list --branch spike-b >buckets-list-child.json
n credentials list --branch spike-b >credentials-list-child.json
neon api "/projects/$P/branches/$spike_b/storage" --output json >api-storage-child.json
n branches delete spike-b >branches-delete.json
n branches delete spike-a >/dev/null

python3 -I "$here/scrub.py" "$raw" "$out"
echo "Review $out before committing; then delete the project: neon projects delete $P"
