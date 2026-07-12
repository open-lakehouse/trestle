-- Initial schema for the Postgres-backed `PgStore`.
--
-- Requires **PostgreSQL 12+ built with ICU** (the official `postgres` Docker
-- images are). The floor is the non-deterministic ICU collation below
-- (`deterministic = false`, added in PG 12); every other feature here (jsonb,
-- GIN `jsonb_path_ops`, `timestamptz`, `bytea`) predates that. Note it is *not*
-- driven by UUIDv7: object/edge ids are minted client-side by the `uuid` crate
-- (`Uuid::new_v4` / `Uuid::now_v7`) and bound as parameters, so `id` is a plain
-- `uuid` column with no `DEFAULT` — the database never generates ids and needs
-- neither the PG 18 native `uuidv7()` nor a hand-rolled PL/pgSQL generator.
--
-- Unlike the SQLite backend (which stores ids/timestamps as TEXT and JSON as
-- TEXT), this schema uses native Postgres types throughout: `uuid` ids,
-- `timestamptz` instants, `bigint` versions, `bytea` for the sealed sensitive
-- blob, and `jsonb` for the object/edge property bags. `jsonb` unlocks a GIN
-- index for containment/path filter pushdown, and CAS is a real
-- `UPDATE … WHERE version = $n RETURNING …`.

-- ASCII case-insensitivity for `name` is provided by SQLite's `COLLATE NOCASE`;
-- Postgres has no built-in equivalent, so we define a non-deterministic ICU
-- collation at case-folding strength (`ks-level2` ignores case and accents).
-- This is a *superset* of SQLite's ASCII-only folding: `Catalog`, `catalog`, and
-- `CATALOG` compare equal, so equality lookups and the uniqueness constraint fold
-- case exactly as the conformance battery requires. Namespace-prefix matching is
-- done in Rust (`ResourceName::prefix_matches`), independent of this collation.
CREATE COLLATION IF NOT EXISTS case_insensitive (
    provider = icu,
    locale = 'und-u-ks-level2',
    deterministic = false
);

CREATE TABLE IF NOT EXISTS objects (
    id          uuid        NOT NULL PRIMARY KEY,
    label       text        NOT NULL,
    name        text        NOT NULL COLLATE case_insensitive,
    properties  jsonb,
    -- Opaque envelope-encrypted blob for the object's sensitive fields, written
    -- atomically with the row (see `ManagedObjectStore`). NULL when the resource
    -- type has no sensitive fields or none were supplied.
    sensitive   bytea,
    version     bigint      NOT NULL DEFAULT 0,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz
);

-- The `name` column already carries the case-insensitive collation, so the
-- unique index folds case and `Catalog`/`catalog` collide on create.
CREATE UNIQUE INDEX IF NOT EXISTS objects_label_name ON objects (label, name);

-- Containment/path filter pushdown (`properties @> …`, `properties -> 'k'`) is
-- served by a GIN index. `jsonb_path_ops` is the smaller, faster operator class
-- for the `@>` containment queries the filter pushdown emits.
CREATE INDEX IF NOT EXISTS objects_properties_gin
    ON objects USING GIN (properties jsonb_path_ops);

CREATE TABLE IF NOT EXISTS associations (
    id          uuid        NOT NULL PRIMARY KEY,
    from_id     uuid        NOT NULL,
    label       text        NOT NULL,
    to_id       uuid        NOT NULL,
    to_label    text        NOT NULL,
    properties  jsonb,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz
);

CREATE UNIQUE INDEX IF NOT EXISTS assoc_from_to_label
    ON associations (from_id, to_id, label);

-- Serves incoming-edge queries (`WHERE to_id = $1 AND label = $2`) and the
-- `OR to_id = $1` branch of object deletion; the trailing `id` keeps the
-- recency `ORDER BY id` index-ordered.
CREATE INDEX IF NOT EXISTS assoc_to_label
    ON associations (to_id, label, id);
