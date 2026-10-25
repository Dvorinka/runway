#!/usr/bin/env bash
# restore.sh — restore a Runway install from a backup.sh archive.
#
#   scripts/restore.sh <archive.tar.gz> [--yes]
#
# Stops the app service, replaces the data dir, reloads Postgres from the
# dump, then restarts. Requires the same install.sh layout (.env +
# compose/production.yml) and the stack running for the DB leg.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USG'
Usage: scripts/restore.sh <archive.tar.gz> [--yes]

Restores a backup produced by scripts/backup.sh. Destructive: the current
data dir and database contents are replaced by the archive's.

  --yes   skip the confirmation prompt
  -h      show this help
USG
  exit 0
}

ARCHIVE=""
YES=0
for arg in "$@"; do
  case "$arg" in
    --yes) YES=1 ;;
    -h|--help) usage ;;
    -*) die "unknown option: $arg (try -h)" ;;
    *) ARCHIVE="$arg" ;;
  esac
done

[[ -n "$ARCHIVE" && -f "$ARCHIVE" ]] || die "backup archive not found — usage: restore.sh <archive.tar.gz>"
[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"

env_val() {
  awk -F= -v k="$1" '$1==k {sub(/^[^=]*=/,""); v=$0} END{print v}' "$ENV_FILE"
}
HOST_DATA_DIR="$(env_val HOST_DATA_DIR)"
[[ -n "$HOST_DATA_DIR" ]] || die "HOST_DATA_DIR missing in $ENV_FILE"
PG_PASS="$(env_val POSTGRES_PASSWORD)"
[[ -n "$PG_PASS" ]] || die "POSTGRES_PASSWORD missing in $ENV_FILE"

docker info >/dev/null 2>&1 || die "docker not reachable"

if [[ "$YES" != 1 ]]; then
  printf 'Restore %s over %s? This replaces the current data and database. [y/N] ' "$ARCHIVE" "$HOST_DATA_DIR"
  read -r reply < /dev/tty || reply=""
  [[ "$reply" =~ ^[Yy]$ ]] || exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Unpacking archive"
tar -xzf "$ARCHIVE" -C "$TMP"
[[ -d "$TMP/data" && -s "$TMP/db/pgdump.sql" ]] || die "not a runway backup archive (expected data/ + db/pgdump.sql)"
[[ -f "$TMP/metadata.json" ]] && cat "$TMP/metadata.json" | tr -d '\n ' && printf '\n'

say "Stopping runway service"
"${COMPOSE[@]}" stop runway >/dev/null

say "Restoring data dir ($HOST_DATA_DIR)"
mkdir -p "$HOST_DATA_DIR"
# Keep a .bak sibling so a bad restore is recoverable without a re-archive.
BAK="${HOST_DATA_DIR%/}.bak.$RANDOM"
mv "$HOST_DATA_DIR" "$BAK" 2>/dev/null || true
mkdir -p "$HOST_DATA_DIR"
if ! tar -C "$TMP/data" -cf - . | tar -C "$HOST_DATA_DIR" -xf -; then
  warn "data copy failed — previous data preserved at $BAK"
fi

say "Restoring database"
PG_CID="$(docker ps -q --filter label=com.docker.compose.service=pgsql | head -n1)"
[[ -n "$PG_CID" ]] || die "pgsql container not running — start the stack first"
# Drop + recreate so stale rows can't survive the restore.
PGPASSWORD="$PG_PASS" "${COMPOSE[@]}" exec -T pgsql \
  psql -U runway -d postgres -qc "DROP DATABASE IF EXISTS runway; CREATE DATABASE runway OWNER runway;"
PGPASSWORD="$PG_PASS" "${COMPOSE[@]}" exec -T pgsql \
  psql -U runway -d runway -q -v ON_ERROR_STOP=1 <"$TMP/db/pgdump.sql"

say "Restarting stack"
"${COMPOSE[@]}" up -d >/dev/null

info "done — previous data dir kept at $BAK"
