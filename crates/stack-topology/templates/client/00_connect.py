"""Connect to the stack.

Lists the client environment this workspace loads (`host.env`, generated beside
`pyproject.toml`) and checks that every endpoint in it answers. Start here: if a
probe fails, the stack is not up (`docker compose up -d --wait`) or the
environment needs a re-render (`trestle env render`).
"""

import marimo

__generated_with = "0.16.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import os
    from pathlib import Path

    import httpx
    import marimo as mo
    from dotenv import dotenv_values

    return Path, dotenv_values, httpx, mo, os


@app.cell
def _(Path, dotenv_values, mo, os):
    # marimo already exported these into `os.environ` (see `[tool.marimo.runtime]` in
    # pyproject.toml); read the file too, to show which variables the stack contributed.
    env_file = Path(__file__).resolve().parent.parent / "host.env"
    client_env = {
        key: os.environ.get(key, value)
        for key, value in dotenv_values(env_file).items()
    }
    mo.vstack(
        [
            mo.md(f"**Client environment** from `{env_file.name}`"),
            mo.ui.table(
                [
                    {"variable": key, "value": value}
                    for key, value in client_env.items()
                ],
                selection=None,
            ),
        ]
    )
    return (client_env,)


@app.cell
def _(client_env, httpx, mo):
    def probe(url):
        # Any HTTP answer means the service is up: many of these are API roots that answer
        # a bare GET with 403/404, and a 401/302 means the stack has sign-in enabled.
        # `verify=False` because an auth-enabled stack serves a locally minted CA.
        try:
            response = httpx.get(url, timeout=3, verify=False)
            return f"reachable (HTTP {response.status_code})"
        except httpx.HTTPError as error:
            return f"unreachable ({type(error).__name__})"

    endpoints = {
        key: value for key, value in client_env.items() if value.startswith("http")
    }
    mo.ui.table(
        [
            {"variable": key, "url": url, "status": probe(url)}
            for key, url in endpoints.items()
        ],
        selection=None,
    )
    return


if __name__ == "__main__":
    app.run()
