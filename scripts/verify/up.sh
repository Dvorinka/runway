#!/usr/bin/env bash
# Brings up the development stack and leaves a seeded, sign-in-able
# dashboard at http://localhost:8000 — Runway's verify-dashboard harness.
#
#   scripts/verify/up.sh           # up + seed
#   scripts/verify/up.sh --fresh   # wipe the db volume first
#
# Requires: docker, docker compose, curl, jq.
set -euo pipefail
cd "$(dirname "$0")/../.."

export HOST_DATA_DIR="$PWD/data-run"
mkdir -p "$HOST_DATA_DIR"

if [[ "${1:-}" == "--fresh" ]]; then
  docker compose -f compose/development.yml down -v
fi

docker compose -f compose/development.yml up -d --build --wait

EMAIL=dev@runway.local
PASS='runway-dev-pass'

# bootstrap is idempotent only for the first user; ignore "already exists".
docker compose -f compose/development.yml exec -T runway \
  runway bootstrap --email "$EMAIL" --password "$PASS" >/dev/null 2>&1 || true

# Seed a demo project so the dashboard has a row to render.
curl -sf -c /tmp/runway-seed.jar -X POST "http://localhost:8000/api/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" >/dev/null

curl -sf -b /tmp/runway-seed.jar -X POST "http://localhost:8000/api/v1/projects" \
  -H 'content-type: application/json' \
  -d '{"name":"demo-app","provider":"upload","repo_full_name":"demo/app","preset":"static"}' \
  >/dev/null 2>&1 || true

echo
echo "  dashboard : http://localhost:8000"
echo "  sign in   : $EMAIL / $PASS"
echo "  seed      : project 'demo-app' (deploy with: runway link + runway deploy)"
echo
