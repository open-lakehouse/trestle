-- The API key tables `PgApiKeyStore` reads, as one deployment defines them
-- (open-lakehouse/luminescent-lakehouse, crates/migrate/migrations/authz/0001_init.sql).
-- Kept in sync by hand; the test substitutes its own schema for `authz`.

CREATE TABLE authz.service_accounts (
    id          uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text        NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9._-]{0,62}$'),
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    disabled_at timestamptz
);

CREATE TABLE authz.api_keys (
    id          text        PRIMARY KEY CHECK (id ~ '^[0-9a-f]{16}$'),
    account_id  uuid        NOT NULL REFERENCES authz.service_accounts (id) ON DELETE CASCADE,
    sha256      bytea       NOT NULL CHECK (length(sha256) = 32),
    scopes      text[]      NOT NULL DEFAULT '{}',
    description text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz,
    revoked_at  timestamptz
);
