"""
Lakehouse quickstart for ``.

This Marimo notebook walks through:

1. Connecting to the local MLflow tracking server (Databricks-shaped URL).
2. Creating a Unity Catalog catalog + schema.
3. Writing a Delta table to SeaweedFS.
4. Logging an MLflow run that references the table.

The same code runs unchanged on Databricks once you deploy.
"""

import marimo


__generated_with = "0.10.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import os

    import marimo as mo

    return mo, os


@app.cell
def _step1(mo, os):
    import mlflow

    mlflow.set_tracking_uri(os.environ["MLFLOW_TRACKING_URI"])
    mlflow.set_experiment(os.environ.get("MLFLOW_EXPERIMENT_NAME", "default"))
    return (mlflow,)


@app.cell
def _step2(os):
    import httpx

    host = os.environ["DATABRICKS_HOST"]
    token = os.environ["DATABRICKS_TOKEN"]
    h = {"Authorization": f"Bearer {token}"}
    httpx.post(
        f"{host}/api/2.1/unity-catalog/catalogs",
        headers=h,
        json={"name": "local_lab"},
    )
    httpx.post(
        f"{host}/api/2.1/unity-catalog/schemas",
        headers=h,
        json={"name": "demo", "catalog_name": "local_lab"},
    )
    return


@app.cell
def _step3(os):
    try:
        import pyarrow as pa
        from deltalake import write_deltalake
    except ImportError:
        return None

    storage_options = {
        "AWS_ENDPOINT_URL": os.environ["MLFLOW_S3_ENDPOINT_URL"],
        "AWS_ACCESS_KEY_ID": os.environ["AWS_ACCESS_KEY_ID"],
        "AWS_SECRET_ACCESS_KEY": os.environ["AWS_SECRET_ACCESS_KEY"],
        "AWS_REGION": os.environ.get("AWS_DEFAULT_REGION", "us-east-1"),
        "AWS_ALLOW_HTTP": "true",
    }
    table = pa.table({"id": [1, 2, 3], "label": ["a", "b", "c"]})
    write_deltalake(
        "s3://unity/local_lab/demo/items",
        table,
        storage_options=storage_options,
        mode="overwrite",
    )
    return (storage_options,)


@app.cell
def _step4(mlflow):
    with mlflow.start_run(run_name="quickstart"):
        mlflow.log_param("table", "local_lab.demo.items")
        mlflow.log_metric("rows", 3)
    return


if __name__ == "__main__":
    app.run()