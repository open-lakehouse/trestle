"""Train a model and track it in MLflow.

Fits a small scikit-learn classifier on the iris dataset. MLflow autologging
records the parameters, metrics and the model itself to the stack's tracking
server (`MLFLOW_TRACKING_URI`, from `host.env`). The tracking server stores the
model artifacts in the stack's object store, so no storage credentials are
needed here.
"""

import marimo

__generated_with = "0.16.0"
app = marimo.App(width="medium")


@app.cell
def _():
    import os

    import marimo as mo
    import mlflow

    # MLflow reads `MLFLOW_TRACKING_URI` from the environment; set it explicitly so a
    # missing `host.env` fails here rather than logging to a local `mlruns/` directory.
    mlflow.set_tracking_uri(os.environ["MLFLOW_TRACKING_URI"])
    experiment = mlflow.set_experiment("trestle-quickstart")
    return experiment, mlflow, mo, os


@app.cell
def _(mo):
    max_depth = mo.ui.slider(1, 8, value=3, label="Tree depth")
    max_depth
    return (max_depth,)


@app.cell
def _(max_depth, mlflow):
    from sklearn.datasets import load_iris
    from sklearn.metrics import accuracy_score
    from sklearn.model_selection import train_test_split
    from sklearn.tree import DecisionTreeClassifier

    X, y = load_iris(return_X_y=True, as_frame=True)
    X_train, X_test, y_train, y_test = train_test_split(
        X, y, test_size=0.3, random_state=42
    )

    # Autologging captures the estimator's parameters, training metrics and the fitted
    # model; the test accuracy is logged by hand.
    mlflow.sklearn.autolog()
    with mlflow.start_run(run_name=f"tree-depth-{max_depth.value}") as run:
        model = DecisionTreeClassifier(max_depth=max_depth.value, random_state=42)
        model.fit(X_train, y_train)
        accuracy = accuracy_score(y_test, model.predict(X_test))
        mlflow.log_metric("test_accuracy", accuracy)
    return accuracy, run


@app.cell
def _(accuracy, experiment, mo, os, run):
    ui = f"{os.environ['MLFLOW_TRACKING_URI']}/mlflow/#/experiments/{experiment.experiment_id}/runs/{run.info.run_id}"
    mo.md(
        f"""
        Test accuracy: **{accuracy:.3f}**

        [Open the run in the MLflow UI]({ui}). Move the slider to log another run, then
        compare them side by side in the experiment view.
        """
    )
    return


if __name__ == "__main__":
    app.run()
