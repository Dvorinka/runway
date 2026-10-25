#!/usr/bin/env bash
# backup.sh — archive a Runway install: Postgres dump + data dir + metadata.
#
#   scripts/backup.sh [--output FILE]
#
# Run from an install produced by install.sh (repo root with .env beside
# compose/production.yml). Output defaults to backups/runway-<ts>.tar.gz
# inside the repo. Re-runs never overwrite an existing archive.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USG'
Usage: scripts/backup.sh [--output FILE] [-h]

Archives the Runway Postgres database and the data dir (uploads, Traefik
config, TLS material) into one tar.gz.

  --output FILE  archive path (default: backups/runway-<timestamp>.tar.gz)
  -h             show this help
USG
  exit 0
}

OUTPUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) OUTPUT="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) die "unknown option: $1 (try -h)" ;;
  esac
done

[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"

env_val() { # env_val KEY -> value from .env (last wins, like dotenv)
  awk -F= -v k="$1" '$1==k {sub(/^[^=]*=/,""); v=$0} END{print v}' "$ENV_FILE"
}

HOST_DATA_DIR="$(env_val HOST_DATA_DIR)"
[[ -n "$HOST_DATA_DIR" && -d "$HOST_DATA_DIR" ]] || die "HOST_DATA_DIR missing or not a directory ($ENV_FILE)"
PG_PASS="$(env_val POSTGRES_PASSWORD)"
[[ -n "$PG_PASS" ]] || die "POSTGRES_PASSWORD missing in $ENV_FILE"

docker info >/dev/null 2>&1 || die "docker not reachable — run as a user that can talk to the daemon"
PG_CID="$(docker ps -q --filter label=com.docker.compose.service=pgsql | head -n1)"
[[ -n "$PG_CID" ]] || die "pgsql container not running — start the stack first"

TS="$(date +%Y%m%d-%H%M%S)"
if [[ -z "$OUTPUT" ]]; then
  mkdir -p "$ROOT/backups"
  OUTPUT="$ROOT/backups/runway-$TS.tar.gz"
fi
[[ ! -e "$OUTPUT" ]] || die "refusing to overwrite existing archive: $OUTPUT"
mkdir -p "$(dirname "$OUTPUT")"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Saving data directory ($HOST_DATA_DIR)"
mkdir -p "$TMP/data"
tar -C "$HOST_DATA_DIR" -cf - . | tar -C "$TMP/data" -xf -

say "Dumping Postgres (runway)"
mkdir -p "$TMP/db"
PGPASSWORD="$PG_PASS" "${COMPOSE[@]}" exec -T pgsql \
  pg_dump -U runway -d runway --no-owner --no-privileges >"$TMP/db/pgdump.sql"
[[ -s "$TMP/db/pgdump.sql" ]] || die "pg_dump produced an empty file"

cat >"$TMP/metadata.json" <<JSON
{
  "created_at": "$(date -Iseconds)",
  "host": "$(hostname 2>/dev/null || echo unknown)",
  "root": "$ROOT",
  "data_dir": "$HOST_DATA_DIR",
  "runway_ref": "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo release)"
}
JSON

say "Packing $OUTPUT"
tar -czf "$OUTPUT" -C "$TMP" data db metadata.json
tar -tzf "$OUTPUT" >/dev/null || die "archive failed verification: $OUTPUT"
chmod 0640 "$OUTPUT" 2>/dev/null || true

info "done — $(du -h "$OUTPUT" | cut -f1) at $OUTPUT"
