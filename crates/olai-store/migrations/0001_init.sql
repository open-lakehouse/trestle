-- Initial schema for the SQLite-backed `SqlStore`.

CREATE TABLE IF NOT EXISTS objects (
    id          TEXT    NOT NULL PRIMARY KEY,
    label       TEXT    NOT NULL,
    -- `COLLATE NOCASE` gives ASCII case-insensitive name matching: equality
    -- lookups (`WHERE name = ?`) and the `objects_label_name` unique index both
    -- fold case, so `Catalog` and `catalog` are the same object and collide on
    -- create. Case folding is ASCII-only (SQLite's built-in NOCASE); non-ASCII
    -- case equivalence is not folded. Callers that need case-insensitive
    -- namespace prefixing get matching semantics from `ResourceName::prefix_matches`.
    name        TEXT    NOT NULL COLLATE NOCASE,
    properties  TEXT,
    -- Opaque envelope-encrypted blob for the object's sensitive fields, written
    -- atomically with the row (see `ManagedObjectStore`). NULL when the resource
    -- type has no sensitive fields or none were supplied.
    sensitive   BLOB,
    version     INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT
);

-- `name COLLATE NOCASE` is explicit here (the column already carries NOCASE, so
-- the index would inherit it, but stating it keeps the case-folding contract
-- legible at the uniqueness constraint that enforces it).
CREATE UNIQUE INDEX IF NOT EXISTS objects_label_name ON objects (label, name COLLATE NOCASE);

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

-- Serves incoming-edge queries (`WHERE to_id = ? AND label = ?`) and the `OR to_id = ?`
-- branch of object deletion; the trailing `id` keeps `ORDER BY id` index-ordered.
CREATE INDEX IF NOT EXISTS assoc_to_label
    ON associations (to_id, label, id);
