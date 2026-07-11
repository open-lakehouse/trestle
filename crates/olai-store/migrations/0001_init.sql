-- Initial schema for the SQLite-backed `SqlStore`.

CREATE TABLE IF NOT EXISTS objects (
    id          TEXT    NOT NULL PRIMARY KEY,
    label       TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    properties  TEXT,
    version     INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS objects_label_name ON objects (label, name);

CREATE TABLE IF NOT EXISTS associations (
    id          TEXT NOT NULL PRIMARY KEY,
    from_id     TEXT NOT NULL,
    label       TEXT NOT NULL,
    to_id       TEXT NOT NULL,
    to_label    TEXT NOT NULL,
    properties  TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS assoc_from_to_label
    ON associations (from_id, to_id, label);
