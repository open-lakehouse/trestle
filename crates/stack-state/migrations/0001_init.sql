-- The shared trestle state, schema version 1. See src/store.rs for what each table holds.

CREATE TABLE workspace (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    mesh        INTEGER NOT NULL,
    vars        TEXT NOT NULL DEFAULT '{}',
    -- Per overlay source, what every instance of the workspace tells it (e.g. which project).
    sources     TEXT NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL
);

CREATE TABLE environment (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    -- The draft: a placement-free env.toml, the module.yaml texts it lists, and the vars a
    -- commit freezes into the revision.
    draft         TEXT NOT NULL,
    draft_modules TEXT NOT NULL DEFAULT '{}',
    draft_vars    TEXT NOT NULL DEFAULT '{}',
    -- Vars every revision of the environment sees, editable at any time.
    vars          TEXT NOT NULL DEFAULT '{}',
    head          INTEGER,
    created_at    INTEGER NOT NULL,
    UNIQUE (workspace_id, name)
);

CREATE TABLE revision (
    environment_id TEXT NOT NULL REFERENCES environment(id) ON DELETE CASCADE,
    number         INTEGER NOT NULL,
    manifest       TEXT NOT NULL,
    modules        TEXT NOT NULL,
    vars           TEXT NOT NULL,
    digest         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (environment_id, number)
);

-- Revisions are immutable: the API never updates one, and this refuses it from anywhere else.
CREATE TRIGGER revision_is_immutable BEFORE UPDATE ON revision
BEGIN
    SELECT RAISE(ABORT, 'revisions are immutable');
END;

CREATE TABLE instance (
    id             TEXT PRIMARY KEY,
    environment_id TEXT NOT NULL,
    revision       INTEGER NOT NULL,
    desired        TEXT NOT NULL,
    overrides      TEXT NOT NULL DEFAULT '{}',
    -- The overlay sources it takes, in order: `[{"name": …, "config": …}]`.
    sources        TEXT NOT NULL DEFAULT '[]',
    created_at     INTEGER NOT NULL,
    FOREIGN KEY (environment_id, revision)
        REFERENCES revision(environment_id, number) ON DELETE CASCADE
);

-- Host-global: one slot per instance, across every workspace on the host.
CREATE TABLE slot_lease (
    slot        INTEGER PRIMARY KEY CHECK (slot BETWEEN 0 AND 54),
    instance_id TEXT NOT NULL UNIQUE REFERENCES instance(id) ON DELETE CASCADE,
    leased_at   INTEGER NOT NULL
);
