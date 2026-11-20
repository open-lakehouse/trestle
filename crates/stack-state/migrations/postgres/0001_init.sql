-- The shared trestle state on Postgres, schema version 1: what a team shares (workspaces,
-- environments, revisions). Instances and slot leases stay in each machine's state.db. The
-- columns match the SQLite tables in ../0001_init.sql and later migrations.

CREATE TABLE state_meta (
    key   TEXT PRIMARY KEY,
    value BIGINT NOT NULL
);

-- `change_seq` counts write statements on the shared tables: a watcher polls it.
INSERT INTO state_meta (key, value) VALUES ('schema_version', 0), ('change_seq', 0);

CREATE TABLE workspace (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    mesh        BOOLEAN NOT NULL,
    vars        TEXT NOT NULL DEFAULT '{}',
    sources     TEXT NOT NULL DEFAULT '{}',
    created_at  BIGINT NOT NULL
);

CREATE TABLE environment (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    draft         TEXT NOT NULL,
    draft_modules TEXT NOT NULL DEFAULT '{}',
    draft_vars    TEXT NOT NULL DEFAULT '{}',
    -- Bumped by every draft edit, so an editor can refuse to overwrite a newer one.
    draft_version BIGINT NOT NULL DEFAULT 0,
    vars          TEXT NOT NULL DEFAULT '{}',
    head          BIGINT,
    created_at    BIGINT NOT NULL,
    UNIQUE (workspace_id, name)
);

CREATE TABLE revision (
    environment_id TEXT NOT NULL REFERENCES environment(id) ON DELETE CASCADE,
    number         BIGINT NOT NULL,
    manifest       TEXT NOT NULL,
    modules        TEXT NOT NULL,
    vars           TEXT NOT NULL,
    digest         TEXT NOT NULL,
    message        TEXT,
    created_at     BIGINT NOT NULL,
    PRIMARY KEY (environment_id, number)
);

CREATE FUNCTION revision_is_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'revisions are immutable';
END
$$;

-- Revisions are immutable: the API never updates one, and this refuses it from anywhere else.
CREATE TRIGGER revision_is_immutable BEFORE UPDATE ON revision
    FOR EACH ROW EXECUTE FUNCTION revision_is_immutable();

CREATE FUNCTION bump_change_seq() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE state_meta SET value = value + 1 WHERE key = 'change_seq';
    RETURN NULL;
END
$$;

CREATE TRIGGER workspace_changed AFTER INSERT OR UPDATE OR DELETE ON workspace
    FOR EACH STATEMENT EXECUTE FUNCTION bump_change_seq();
CREATE TRIGGER environment_changed AFTER INSERT OR UPDATE OR DELETE ON environment
    FOR EACH STATEMENT EXECUTE FUNCTION bump_change_seq();
CREATE TRIGGER revision_changed AFTER INSERT OR UPDATE OR DELETE ON revision
    FOR EACH STATEMENT EXECUTE FUNCTION bump_change_seq();
