"""STS front for RustFS: rewrites session policies RustFS can't parse, then re-signs.

Unity Catalog's AssumeRole session policy uses mid-word action wildcards
(``s3:GetO*``, ``s3:*Multipart*``). AWS accepts them; RustFS only parses exact
action names and ``s3:*``, and rejects the whole request. The body is covered by
the caller's SigV4 signature, so a proxy can't edit it in flight: this shim
accepts the request, expands the wildcards against the actions RustFS knows,
drops statements whose condition keys RustFS rejects, and forwards a freshly
signed AssumeRole using its own RustFS credentials.

The caller's signature is not verified. The shim is reachable only on the
compose network, behind Envoy's ``sts.*.amazonaws.com`` route.
"""

from __future__ import annotations

import datetime
import fnmatch
import hashlib
import hmac
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = os.environ.get("STS_UPSTREAM", "http://rustfs:9000")
ACCESS_KEY = os.environ["STS_ACCESS_KEY"]
SECRET_KEY = os.environ["STS_SECRET_KEY"]
REGION = os.environ.get("AWS_REGION", "us-east-1")

# Every object-level action RustFS 1.0 accepts in a policy (probed one by one;
# e.g. GetObjectTorrent and PutObjectVersionAcl are rejected).
RUSTFS_S3_ACTIONS = [
    "s3:AbortMultipartUpload",
    "s3:DeleteObject",
    "s3:DeleteObjectTagging",
    "s3:DeleteObjectVersion",
    "s3:DeleteObjectVersionTagging",
    "s3:GetObject",
    "s3:GetObjectAcl",
    "s3:GetObjectAttributes",
    "s3:GetObjectLegalHold",
    "s3:GetObjectRetention",
    "s3:GetObjectTagging",
    "s3:GetObjectVersion",
    "s3:GetObjectVersionAttributes",
    "s3:GetObjectVersionForReplication",
    "s3:GetObjectVersionTagging",
    "s3:ListBucket",
    "s3:ListBucketMultipartUploads",
    "s3:ListMultipartUploadParts",
    "s3:PutObject",
    "s3:PutObjectAcl",
    "s3:PutObjectLegalHold",
    "s3:PutObjectRetention",
    "s3:PutObjectTagging",
    "s3:PutObjectVersionTagging",
]


def _expand(action: str) -> list[str]:
    if "*" not in action or action in ("*", "s3:*"):
        return [action]
    return [a for a in RUSTFS_S3_ACTIONS if fnmatch.fnmatchcase(a, action)]


def _condition_keys(statement: dict) -> set[str]:
    return {k for clause in statement.get("Condition", {}).values() for k in clause}


def rewrite_policy(policy: str) -> str:
    doc = json.loads(policy)
    statements = []
    for st in doc.get("Statement", []):
        # RustFS has no KMS: kms:* statements (and their kms:* condition keys)
        # can only narrow, so dropping them grants nothing extra.
        if any(k.startswith("kms:") for k in _condition_keys(st)):
            continue
        actions = st.get("Action", [])
        if isinstance(actions, str):
            actions = [actions]
        st["Action"] = sorted({x for a in actions for x in _expand(a)})
        if st["Action"]:
            statements.append(st)
    doc["Statement"] = statements
    return json.dumps(doc)


def _sign(body: bytes, host: str) -> dict[str, str]:
    """SigV4 headers for a form-encoded STS POST to ``/``."""
    now = datetime.datetime.now(datetime.UTC)
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    day = now.strftime("%Y%m%d")
    payload_hash = hashlib.sha256(body).hexdigest()
    headers = {
        "content-type": "application/x-www-form-urlencoded; charset=utf-8",
        "host": host,
        "x-amz-content-sha256": payload_hash,
        "x-amz-date": amz_date,
    }
    signed = ";".join(sorted(headers))
    canonical = "\n".join(
        [
            "POST",
            "/",
            "",
            "".join(f"{k}:{headers[k]}\n" for k in sorted(headers)),
            signed,
            payload_hash,
        ]
    )
    scope = f"{day}/{REGION}/sts/aws4_request"
    to_sign = "\n".join(
        [
            "AWS4-HMAC-SHA256",
            amz_date,
            scope,
            hashlib.sha256(canonical.encode()).hexdigest(),
        ]
    )
    key = f"AWS4{SECRET_KEY}".encode()
    for part in (day, REGION, "sts", "aws4_request"):
        key = hmac.new(key, part.encode(), hashlib.sha256).digest()
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    headers["authorization"] = (
        f"AWS4-HMAC-SHA256 Credential={ACCESS_KEY}/{scope}, "
        f"SignedHeaders={signed}, Signature={signature}"
    )
    return headers


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        # Health probe only; real STS calls are POSTs.
        self.send_response(200)
        self.end_headers()

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        form = urllib.parse.parse_qs(raw.decode(), keep_blank_values=True)
        if "Policy" in form:
            form["Policy"] = [rewrite_policy(form["Policy"][0])]
        body = urllib.parse.urlencode(form, doseq=True).encode()
        host = urllib.parse.urlsplit(UPSTREAM).netloc
        req = urllib.request.Request(
            f"{UPSTREAM}/", data=body, headers=_sign(body, host), method="POST"
        )
        try:
            with urllib.request.urlopen(req) as resp:
                status, ctype, out = (
                    resp.status,
                    resp.headers.get("content-type"),
                    resp.read(),
                )
        except urllib.error.HTTPError as err:
            status, ctype, out = err.code, err.headers.get("content-type"), err.read()
        self.send_response(status)
        self.send_header("content-type", ctype or "text/xml")
        self.send_header("content-length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, fmt, *args):
        if self.command == "GET":
            return
        sys.stderr.write(f"sts-shim: {fmt % args}\n")


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
