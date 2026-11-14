-- The permission data the services of a deployment share: who the machine callers
-- are, how they authenticate, the groups, and the grants. Users come from the
-- identity provider; this schema refers to them by their subject.
--
-- Names are unqualified: `olai_auth::migrate` runs this with `search_path` set to the
-- target schema (`authz` by default), which also holds the migration ledger.
--
-- Subjects are typed: `user` (an identity provider subject), `service_account` and
-- `group` (ids from the tables below). Objects are Cedar entities: an entity type such
-- as `Ns::Folder` and an id. Privileges are names the Cedar policies give meaning to.

CREATE TABLE service_accounts (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text        NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,62}$'),
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    -- A disabled account authenticates with nothing.
    disabled_at timestamptz
);

-- `olk_<id>_<secret>`: `id` is public (16 hex characters, the lookup key), the
-- secret is 32 random bytes as hex. Only the SHA-256 of the whole key string is
-- stored; the key itself is shown once, when it is issued.
CREATE TABLE api_keys (
    id          text        PRIMARY KEY CHECK (id ~ '^[0-9a-f]{16}$'),
    account_id  uuid        NOT NULL REFERENCES service_accounts (id) ON DELETE CASCADE,
    sha256      bytea       NOT NULL CHECK (length(sha256) = 32),
    scopes      text[]      NOT NULL DEFAULT '{}',
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz,
    revoked_at  timestamptz
);
CREATE INDEX api_keys_account ON api_keys (account_id);

CREATE TABLE groups (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text        NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,62}$'),
    description text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- Groups can contain groups; the entity loader walks the parents.
CREATE TABLE memberships (
    group_id       uuid        NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    principal_type text        NOT NULL CHECK (principal_type IN ('user', 'service_account', 'group')),
    principal_id   text        NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, principal_type, principal_id),
    CHECK (NOT (principal_type = 'group' AND principal_id = group_id::text))
);
CREATE INDEX memberships_principal ON memberships (principal_type, principal_id);

-- (subject, privilege, object): the grants.
CREATE TABLE grants (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_type text        NOT NULL CHECK (subject_type IN ('user', 'service_account', 'group')),
    subject_id   text        NOT NULL,
    privilege    text        NOT NULL CHECK (privilege ~ '^[A-Za-z][A-Za-z0-9_]{0,62}$'),
    object_type  text        NOT NULL CHECK (object_type ~ '^[A-Z][A-Za-z0-9_]*(::[A-Z][A-Za-z0-9_]*)+$'),
    object_id    text        NOT NULL CHECK (object_id <> ''),
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (subject_type, subject_id, privilege, object_type, object_id)
);
CREATE INDEX grants_object ON grants (object_type, object_id);
CREATE INDEX grants_subject ON grants (subject_type, subject_id);

-- `authz_reader`: what a service needs to authenticate and authorize, and no more. A
-- login role the service connects as joins it (`GRANT authz_reader TO ...`). Roles are
-- per cluster, so it may exist already (another schema's or database's migration).
DO $$
BEGIN
    CREATE ROLE authz_reader NOLOGIN;
EXCEPTION
    WHEN duplicate_object OR unique_violation THEN NULL;
END
$$;
DO $$
BEGIN
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO authz_reader', current_schema());
    EXECUTE format('GRANT SELECT ON ALL TABLES IN SCHEMA %I TO authz_reader', current_schema());
    EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA %I GRANT SELECT ON TABLES TO authz_reader', current_schema());
END
$$;
