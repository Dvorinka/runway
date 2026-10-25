#!/usr/bin/env bash
# restart.sh — restart the Runway stack in place (no rebuild).
#
#   scripts/restart.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon"

say "Restarting"
(cd "$ROOT" && "${COMPOSE[@]}" restart)

say "Waiting for the app"
for _ in $(seq 1 60); do
  (cd "$ROOT" && "${COMPOSE[@]}" exec -T runway true) >/dev/null 2>&1 && break
  sleep 2
done
(cd "$ROOT" && "${COMPOSE[@]}" exec -T runway true) >/dev/null 2>&1 || \
  die "app did not come up — check: docker compose -f compose/production.yml logs runway"

say "Back up"
