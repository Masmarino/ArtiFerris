#!/usr/bin/env bash
# Runs the backend and the frontend locally, with hot reload. Postgres comes from docker-compose and is left running
# on Ctrl+C. The backend is a local `cargo run` on port 8081, not the compose container.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Same POSTGRES_PASSWORD as docker-compose, so the local backend can connect.
if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

export DATABASE_URL="postgres://artiferris:${POSTGRES_PASSWORD:-artiferris}@localhost:5432/artiferris"
export JWT_SECRET="local-dev-secret-not-for-production"
# Fixed so secrets in the dev database stay readable across restarts; not taken from .env, whose value is the placeholder.
export SECRETS_ENCRYPTION_KEY="local-dev-secrets-key-not-for-production"
export ARTIFERRIS_BASE_DOMAIN="${ARTIFERRIS_BASE_DOMAIN:-artiferris.localhost}"
export BIND_ADDR="0.0.0.0:8081"
# The browser only talks to the Angular dev server on 4200. Without this, the WebAuthn origin would be computed from
# port 8081 and every passkey ceremony would fail with InvalidRPOrigin. Not PUBLIC_URL itself: .env carries the
# production one.
export PUBLIC_URL="${DEV_PUBLIC_URL:-http://localhost:4200}"
export STORAGE_ROOT="$ROOT_DIR/data"
export ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME="admin"
export ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD="admin123"
export RUST_LOG="${RUST_LOG:-info}"

echo "==> Starting Postgres (docker-compose, port 5432)"
docker compose up -d --wait postgres

echo "==> Running migrations"
sqlx migrate run --source crates/artiferris-infrastructure/migrations

echo "==> Starting backend (cargo run -p artiferris-api, port 8081)"
cargo run -p artiferris-api &
BACKEND_PID=$!

CLEANED_UP=0
cleanup() {
  [ "$CLEANED_UP" = 1 ] && return
  CLEANED_UP=1
  echo
  echo "==> Stopping backend"
  kill "$BACKEND_PID" 2>/dev/null || true
  wait "$BACKEND_PID" 2>/dev/null || true
  # Postgres stays up: `docker compose stop postgres` to stop it.
  exit 0
}
trap cleanup EXIT INT TERM

echo "==> Starting frontend (npm start, port 4200, hot reload)"
npm start --prefix frontend
