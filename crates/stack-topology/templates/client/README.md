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

If the gateway requires sign-in (`envoy.auth`), the examples don't authenticate
yet. Use a stack without auth for now.
