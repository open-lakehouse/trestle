#!/bin/bash
# Runs once on first Postgres startup (empty data dir). Creates each database
# declared by a component under `provides.postgres_databases`.
set -euo pipefail

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-SQL
    CREATE DATABASE unitycatalog;
    GRANT ALL PRIVILEGES ON DATABASE unitycatalog TO $POSTGRES_USER;
    CREATE DATABASE mlflow;
    GRANT ALL PRIVILEGES ON DATABASE mlflow TO $POSTGRES_USER;
SQL