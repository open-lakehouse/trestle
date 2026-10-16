"""Define and materialize assets with the Rivers SDK.

Assets are functions decorated with `@rs.Asset`; a parameter named after another asset is a
dependency. This notebook runs a small repository in-process. The stack's Rivers server
(`RIVERS_URL`, from `host.env`) serves the operator-owned code location in
`pipelines/rivers/` beside the stack's `compose.yaml`, where its runs are scheduled and
followed in the web UI.
"""

import marimo

__generated_with = "0.16.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import os

    import marimo as mo
    import rivers as rs

    mo.md(
        f"""
        Rivers web UI: [{os.environ["RIVERS_URL"]}]({os.environ["RIVERS_URL"]}). Its
        pipelines live in `pipelines/rivers/`; reload the code location there after editing.
        """
    )
    return mo, rs


@app.cell
def _(rs):
    @rs.Asset
    def users() -> list[dict]:
        return [
            {"id": 1, "name": "Alice", "active": True},
            {"id": 2, "name": "Bob", "active": False},
            {"id": 3, "name": "Carol", "active": True},
        ]

    @rs.Asset
    def active_users(users: list[dict]) -> list[dict]:
        return [u for u in users if u["active"]]

    @rs.Asset
    def active_user_count(active_users: list[dict]) -> int:
        return len(active_users)

    # In-process: the notebook's assets can't be shipped to a subprocess pool.
    repo = rs.CodeRepository(
        assets=[users, active_users, active_user_count],
        default_executor=rs.Executor.in_process(),
    )
    return (repo,)


@app.cell
def _(mo, repo):
    result = repo.materialize()
    mo.md(
        f"""
        Run `{result.run_id}`: success `{result.success}`, materialized
        `{sorted(result.materialized_assets)}`.

        Active users: `{repo.load_node("active_user_count")}`
        """
    )
    return


if __name__ == "__main__":
    app.run()
