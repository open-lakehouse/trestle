# /// script
# requires-python = ">=3.11"
# dependencies = ["mcp>=1.12,<2", "uvicorn>=0.30"]
# ///
"""A tiny MCP server (Streamable HTTP) that answers only with `Authorization: Bearer $MCP_TOKEN`.

The live check for Omnigent's MCP registry puts it behind an egress route: Omnigent calls the
egress gateway with no credential, and the gateway adds the bearer. A request without it, or
with another token, gets a 401, so a listed tool proves the gateway injected it.

    MCP_TOKEN=... MCP_TLS_CERT=cert.pem MCP_TLS_KEY=key.pem uv run server.py

It serves `/mcp` on `MCP_PORT` (default 8443), over TLS when both `MCP_TLS_*` paths are set.
"""

import hmac
import os

import uvicorn
from mcp.server.fastmcp import FastMCP

TOKEN = os.environ["MCP_TOKEN"]

mcp = FastMCP("bearer-test", stateless_http=True, json_response=True)


@mcp.tool()
def echo(text: str) -> str:
    """Return `text` unchanged."""
    return text


@mcp.tool()
def add(a: int, b: int) -> int:
    """Add two integers."""
    return a + b


def require_bearer(app):
    expected = f"Bearer {TOKEN}".encode()

    async def guarded(scope, receive, send):
        if scope["type"] == "http":
            got = dict(scope["headers"]).get(b"authorization", b"")
            if not hmac.compare_digest(got, expected):
                await send(
                    {
                        "type": "http.response.start",
                        "status": 401,
                        "headers": [(b"content-type", b"text/plain")],
                    }
                )
                await send({"type": "http.response.body", "body": b"unauthorized\n"})
                return
        await app(scope, receive, send)

    return guarded


if __name__ == "__main__":
    # The egress forwards the route's own host; accept whatever it names.
    mcp.settings.transport_security = None
    cert, key = os.environ.get("MCP_TLS_CERT"), os.environ.get("MCP_TLS_KEY")
    uvicorn.run(
        require_bearer(mcp.streamable_http_app()),
        host="0.0.0.0",
        port=int(os.environ.get("MCP_PORT", "8443")),
        ssl_certfile=cert,
        ssl_keyfile=key,
    )
