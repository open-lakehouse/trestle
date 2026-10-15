# Client workspace

A [uv](https://docs.astral.sh/uv/) project with [marimo](https://marimo.io) notebooks
that talk to this environment. `trestle env` generates it. `pyproject.toml` and this
README are rewritten on every render; notebooks are written once, so your edits stay.

```bash
docker compose up -d --wait        # from the environment root
cd client
uv sync                            # create .venv with the stack's client packages
uv run marimo edit notebooks       # open the notebooks in your browser
```

Notebooks read their endpoints from `host.env`, which marimo loads automatically
(`[tool.marimo.runtime] dotenv` in `pyproject.toml`). It holds addresses only, no
credentials. To run a notebook from a container on the compose network, point
`dotenv` at `container.env` instead.

## Hosted editor

Select the `marimo` module and the stack serves this workspace itself, at
`$MARIMO_URL` (`<gateway>/marimo/`), with no local Python or uv needed. It edits
these same files. Inside the container the notebooks load `container.env` in
place of `host.env`, and packages install into a container-owned virtualenv
(the host's `.venv` is untouched). The editor has no token of its own: the
gateway, and its sign-in when enabled, is the only thing guarding it, and anyone
who reaches it can run code in the stack.

The container installs from PyPI and doesn't see your host's uv config. Where
PyPI is unreachable (e.g. a mirror is configured in `uv.toml`), set the
`marimo.index_url` knob to the mirror's `/simple/` URL, or the editor won't start.

If the gateway requires sign-in (`envoy.auth`), the examples don't authenticate
yet. Use a stack without auth for now.
