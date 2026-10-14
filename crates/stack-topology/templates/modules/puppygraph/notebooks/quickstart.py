"""Query the lakehouse as a graph with PuppyGraph.

PuppyGraph reads Unity Catalog's tables in place: `graphs/puppygraph/schema.json` (beside the
stack's `compose.yaml`) says which tables are nodes and edges. Once it maps some, this
notebook runs the same question over openCypher (Bolt, `PUPPYGRAPH_BOLT_ADDRESS`) and Gremlin
(`PUPPYGRAPH_GREMLIN_ADDRESS`), both from `host.env`.

The client environment carries no credentials: the login defaults to the stack's local
`puppygraph` user; set `PUPPYGRAPH_USERNAME` / `PUPPYGRAPH_PASSWORD` if you changed it.
"""

import marimo

__generated_with = "0.16.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import os

    import marimo as mo

    user = os.environ.get("PUPPYGRAPH_USERNAME", "puppygraph")
    password = os.environ.get("PUPPYGRAPH_PASSWORD", "puppygraph123")
    mo.md(
        f"""
        Graph schema and web UI: [{os.environ["PUPPYGRAPH_URL"]}]({os.environ["PUPPYGRAPH_URL"]})
        (sign in as `{user}`).
        """
    )
    return mo, os, password, user


@app.cell
def _(mo, os, password, user):
    from neo4j import GraphDatabase

    # openCypher over Bolt: count the nodes per label.
    with GraphDatabase.driver(
        f"bolt://{os.environ['PUPPYGRAPH_BOLT_ADDRESS']}", auth=(user, password)
    ) as driver:
        records, _, _ = driver.execute_query(
            "MATCH (n) RETURN labels(n)[0] AS label, count(*) AS nodes ORDER BY label"
        )
    cypher_counts = {r["label"]: r["nodes"] for r in records}
    mo.md(f"Cypher, nodes per label: `{cypher_counts}`")
    return (cypher_counts,)


@app.cell
def _(mo, os, password, user):
    from gremlin_python.driver import client, serializer

    # The same question in Gremlin, over its WebSocket endpoint.
    gremlin = client.Client(
        f"ws://{os.environ['PUPPYGRAPH_GREMLIN_ADDRESS']}/gremlin",
        "g",
        username=user,
        password=password,
        message_serializer=serializer.GraphSONSerializersV3d0(),
    )
    try:
        gremlin_counts = gremlin.submit("g.V().groupCount().by(label)").all().result()
    finally:
        gremlin.close()
    mo.md(f"Gremlin, vertices per label: `{gremlin_counts}`")
    return (gremlin_counts,)


if __name__ == "__main__":
    app.run()
