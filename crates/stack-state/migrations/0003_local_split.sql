-- Schema version 3: this file's machine-local half can serve a shared store elsewhere.
--
-- An instance may run a revision of an environment held in an attached Postgres database
-- (`store` names the attachment, NULL means this file's tables), so it no longer references
-- the revision by foreign key: the store checks the revision exists when it creates one, and
-- deletes an environment's instances with it. `migrate` runs this with foreign keys off, so
-- rebuilding `instance` doesn't cascade into `slot_lease`.

-- The shared databases attached on this machine: where each one is, never a credential.
CREATE TABLE attachment (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    -- `{"neon": {...}}`: see `RemoteLocation`.
    location    TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL
);

CREATE TABLE instance_new (
    id             TEXT PRIMARY KEY,
    store          TEXT REFERENCES attachment(id),
    environment_id TEXT NOT NULL,
    revision       INTEGER NOT NULL,
    desired        TEXT NOT NULL,
    overrides      TEXT NOT NULL DEFAULT '{}',
    sources        TEXT NOT NULL DEFAULT '[]',
    created_at     INTEGER NOT NULL
);

INSERT INTO instance_new (id, store, environment_id, revision, desired, overrides, sources,
                          created_at)
    SELECT id, NULL, environment_id, revision, desired, overrides, sources, created_at
    FROM instance;

DROP TABLE instance;
ALTER TABLE instance_new RENAME TO instance;
CREATE INDEX instance_environment ON instance (environment_id);

-- This machine's own vars for an environment, wherever the environment is held: the minted
-- keys sealing its instances' data, and the user's own secrets. Never shared.
CREATE TABLE local_var (
    environment_id TEXT PRIMARY KEY,
    vars           TEXT NOT NULL DEFAULT '{}'
);

-- Bumped by every draft edit, so an editor can refuse to overwrite a newer one.
ALTER TABLE environment ADD COLUMN draft_version INTEGER NOT NULL DEFAULT 0;
