run +cmd:
  cargo run --bin trestle {{ cmd }}

# Regenerate the committed `olai-store/.sqlx/` offline query cache for BOTH sqlx
# backends (SQLite + Postgres).
#
# `cargo sqlx prepare` rewrites the whole `.sqlx/` directory from the queries in a
# single `cargo check`, and a `query!` macro is dialect-bound: SQLite queries can
# only be verified against SQLite and Postgres queries only against Postgres.
# There is no single build that checks both, so we prepare each dialect against
# its own database and take the *union* — safe because every cache entry is a
# hash-named, self-describing (`"db_name"`) file, so the two dialects never
# collide.
#
# Needs sqlx-cli (`cargo install sqlx-cli`) and a reachable Postgres. Connection
# parts default to the throwaway `just test-pg` container; override positionally,
# e.g. `just regen-sqlx myhost 5432 myuser mypass mydb`.
regen-sqlx PG_HOST="localhost" PG_PORT="5433" PG_USER="postgres" PG_PASS="postgres" PG_DB="olai_store":
  #!/usr/bin/env bash
  set -euo pipefail
  cd crates/olai-store
  pg_url="postgres://{{ PG_USER }}:{{ PG_PASS }}@{{ PG_HOST }}:{{ PG_PORT }}/{{ PG_DB }}"
  sqlite_file="$(mktemp -t olai_store_sqlx.XXXXXX).db"
  stash="$(mktemp -d)"
  trap 'rm -rf "$stash" "$sqlite_file"' EXIT

  echo ">> Postgres pass ($pg_url)"
  DATABASE_URL="$pg_url" sqlx migrate run --source ./migrations/postgres
  DATABASE_URL="$pg_url" SQLX_OFFLINE=false \
    cargo sqlx prepare -- --no-default-features --features postgres --lib
  # Stash the Postgres entries before the SQLite pass overwrites `.sqlx/`.
  cp .sqlx/*.json "$stash/"

  echo ">> SQLite pass"
  DATABASE_URL="sqlite://${sqlite_file}" sqlx database create
  DATABASE_URL="sqlite://${sqlite_file}" sqlx migrate run --source ./migrations/sqlite
  DATABASE_URL="sqlite://${sqlite_file}" SQLX_OFFLINE=false \
    cargo sqlx prepare -- --no-default-features --features sqlite --lib

  echo ">> Union: fold the Postgres entries back in"
  cp "$stash"/*.json .sqlx/

  echo ">> Done: $(ls .sqlx/*.json | wc -l | tr -d ' ') entries" \
       "(SQLite $(grep -l '"SQLite"' .sqlx/*.json | wc -l | tr -d ' ')," \
       "PostgreSQL $(grep -l '"PostgreSQL"' .sqlx/*.json | wc -l | tr -d ' '))"

# Spin up a throwaway Postgres in Docker and run the olai-store Postgres
# conformance battery against it. Mirrors the `postgres` CI job locally.
test-pg PG_PORT="5433":
  #!/usr/bin/env bash
  set -euo pipefail
  name=olai-store-pg-test
  user=postgres pass=postgres db=olai_store
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker run -d --name "$name" \
    -e POSTGRES_USER="$user" -e POSTGRES_PASSWORD="$pass" -e POSTGRES_DB="$db" \
    -p {{ PG_PORT }}:5432 postgres:16 >/dev/null
  trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
  echo ">> waiting for postgres..."
  for _ in $(seq 1 30); do
    docker exec "$name" pg_isready -U "$user" -d "$db" >/dev/null 2>&1 && break
    sleep 1
  done
  export DATABASE_URL_PG="postgres://${user}:${pass}@localhost:{{ PG_PORT }}/${db}"
  SQLX_OFFLINE=true cargo test --locked --lib -p olai-store --features postgres

# Regenerate the Homebrew formula for an existing `olai-trestle-v*` release and
# print it (does not push). Downloads the `.sha256` sidecars from that release
# and renders `Formula/trestle.rb` via scripts/gen-homebrew-formula.sh — the same
# path the `bump-homebrew` CI job takes. Useful for eyeballing / `brew style`ing
# the formula locally before cutting a release. Needs `gh` authenticated.
#   just homebrew-formula olai-trestle-v0.0.5
homebrew-formula TAG:
  #!/usr/bin/env bash
  set -euo pipefail
  repo=open-lakehouse/trestle
  version="${TAG#olai-trestle-v}"
  d="$(mktemp -d)"
  trap 'rm -rf "$d"' EXIT
  echo ">> downloading checksums for {{ TAG }}" >&2
  gh release download "{{ TAG }}" --repo "$repo" --dir "$d" --pattern '*.sha256'
  scripts/gen-homebrew-formula.sh \
    --tool trestle --crate olai-trestle --version "$version" \
    --repo "$repo" --tag-prefix olai-trestle-v \
    --desc "Unified CLI for proto-driven code generation and full-project scaffolding" \
    --homepage https://github.com/open-lakehouse/trestle \
    --checksums-dir "$d"
