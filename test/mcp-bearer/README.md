# `mcp-bearer`: an MCP server that needs a bearer token

`server.py` is a tiny MCP server (Streamable HTTP at `/mcp`, tools `echo` and `add`) that
answers only with `Authorization: Bearer $MCP_TOKEN`, over TLS when `MCP_TLS_CERT` and
`MCP_TLS_KEY` are set. It is the upstream for the live check of Omnigent's MCP registry. The
egress gateway adds the bearer, so a listed tool shows that the gateway injected it.

## Live check: Omnigent through an egress route

Run this with Docker (colima: `export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock`).
Keep the scratch directory under `$HOME`, because Docker can't bind-mount from `/tmp`.

1. In a scratch dir, mint a throwaway CA and a server certificate for `mcp-bearer`:

   ```bash
   openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=test CA" -keyout ca.key -out ca.pem
   openssl req -newkey rsa:2048 -nodes -subj "/CN=mcp-bearer" -keyout key.pem -out srv.csr
   printf "subjectAltName=DNS:mcp-bearer\nextendedKeyUsage=serverAuth\n" > ext.cnf
   openssl x509 -req -in srv.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 2 -extfile ext.cnf -out cert.pem
   chmod 644 key.pem
   ```

2. Write an `env.toml` with Omnigent and one MCP route to the server, then create the
   environment, commit it and set the route's secret:

   ```toml
   version = 8

   [selection]
   modules = ["omnigent"]

   [selection.knob_overrides.omnigent]
   upstream = "databricks"   # or another upstream you have credentials for

   [[selection.egress.routes]]
   name = "bearer-test"
   upstream = "https://mcp-bearer:8443"
   path = "/mcp"
   consumers = ["omnigent"]
   credential = { kind = "bearer", secret = "MCP_TEST_TOKEN" }
   mcp = { title = "Bearer test", tools = ["echo", "add"] }
   ```

   ```bash
   export TRESTLE_HOME=$PWD/home
   trestle ws create live && trestle env create live/mcp --from env.toml && trestle env commit live/mcp
   printf %s "$SENTINEL" | trestle var set --secret live/mcp MCP_TEST_TOKEN
   trestle up live/mcp
   ```

3. Run the server on the environment's network as `mcp-bearer`. Packages come from a PyPI
   mirror when pypi.org is blocked:

   ```bash
   docker run -d --name mcp-bearer --network <project>_default --network-alias mcp-bearer \
     -e MCP_TOKEN="$SENTINEL" -e MCP_TLS_CERT=/certs/cert.pem -e MCP_TLS_KEY=/certs/key.pem \
     -e UV_INDEX_URL="$MIRROR" -v "$PWD:/certs:ro" -v "<repo>/test/mcp-bearer:/srv:ro" \
     ghcr.io/astral-sh/uv:python3.12-bookworm-slim uv run /srv/server.py
   ```

4. Make the egress Envoy trust the test CA, since it verifies upstreams against the system
   store:

   ```bash
   E=<project>-egress-gateway-1
   docker exec $E cat /etc/ssl/certs/ca-certificates.crt > bundle.pem && cat ca.pem >> bundle.pem
   docker cp bundle.pem $E:/etc/ssl/certs/ca-certificates.crt && docker restart $E
   ```

5. Check it through Omnigent (`B=http://localhost:<gateway port>/omnigent`):
   - `GET $B/v1/mcp-registry/services` lists `bearer-test` as connected.
   - `POST $B/v1/sessions` with `{"agent_id", "host_id", "workspace": "/workspace",
     "mcp_registry_services": ["bearer-test"]}` creates a session.
   - `POST $B/v1/mcp/bearer-test` with `X-Omnigent-Session-Id: <session>` answers
     `initialize`, `tools/list` (`echo`, `add`) and `tools/call`.
   - `omnigent run --server http://omnigent:8000 --resume <session> -p "use the add tool …"`,
     run in the host container, makes the agent call `bearer-test__add`.
   - The server logs `200` only for requests from the egress gateway's IP. A request from the
     Omnigent container without the bearer gets `401`.
   - The sentinel appears in no session or agent spec, in no `docker inspect` of `omnigent`,
     `omnigent-host` or `egress-gateway`, in no file under their data, and nowhere in the
     instance directory. Only `secret-server`'s environment holds it.

6. Clean up: `trestle down live`, `docker rm -f mcp-bearer`, remove the keychain entry
   (`security delete-generic-password -s trestle -a <ws-id>/MCP_TEST_TOKEN` on macOS), and
   delete the scratch dir.
