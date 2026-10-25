#!/usr/bin/env bash
# uninstall.sh — remove a Runway install from this machine.
#
#   scripts/uninstall.sh [--yes] [--keep-data] [--skip-backup]
#
# Stops the stack, drops the Postgres volume, removes Runway-managed
# deployment containers, then the data dir and .env. The repo checkout
# itself is left in place — delete it manually when done.
# This is destructive. A backup runs first unless --skip-backup.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USG'
Usage: scripts/uninstall.sh [--yes] [--keep-data] [--skip-backup]

Removes the stack (containers + network + Postgres volume), all
Runway-managed deployment containers, the data dir, and .env.

  --yes, -y      skip the confirmation prompt
  --keep-data    leave .env and the data dir behind
  --skip-backup  do not run scripts/backup.sh first
  -h             show this help
USG
  exit 0
}

YES=0; KEEP_DATA=0; SKIP_BACKUP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes|-y) YES=1; shift ;;
    --keep-data) KEEP_DATA=1; shift ;;
    --skip-backup) SKIP_BACKUP=1; shift ;;
    -h|--help) usage ;;
    *) die "unknown option: $1 (try -h)" ;;
  esac
done

[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"
docker info >/dev/null 2>&1 || die "docker not reachable — run as a user that can talk to the daemon"

env_val() { awk -F= -v k="$1" '$1==k {sub(/^[^=]*=/,""); v=$0} END{print v}' "$ENV_FILE"; }
HOST="$(env_val APP_HOSTNAME)"; HOST="${HOST:-this Runway install}"
DATA_DIR="$(env_val HOST_DATA_DIR)"

if [[ $YES -eq 0 ]]; then
  printf 'This removes the stack, deployment containers, and %s.\n' \
    "${DATA_DIR:-the data dir}"
  printf 'Type %s to confirm: ' "$HOST"
  read -r answer
  [[ "$answer" == "$HOST" ]] || die "aborted — typed '$answer', expected '$HOST'"
fi

if [[ $SKIP_BACKUP -eq 0 && -x "$ROOT/scripts/backup.sh" ]] \
  && docker ps -q --filter label=com.docker.compose.service=pgsql | grep -q .; then
  say "Backing up first (—skip-backup to skip)"
  "$ROOT/scripts/backup.sh"
fi

say "Stopping the stack"
"${COMPOSE[@]}" down -v --remove-orphans \
  || printf '\033[1;33mwarn:\033[0m compose down failed — continuing\n' >&2

# Deployment containers aren't in the compose project — drop them by label.
mapfile -t managed < <(docker ps -aq --filter label=runway.managed)
if [[ ${#managed[@]} -gt 0 ]]; then
  say "Removing ${#managed[@]} Runway-managed deployment container(s)"
  docker rm -f "${managed[@]}" >/dev/null
fi
if docker ps -aq --filter name='^/cloudflared-instance$' | grep -q .; then
  docker rm -f cloudflared-instance >/dev/null
fi

if [[ $KEEP_DATA -eq 0 ]]; then
  say "Removing data dir and .env"
  [[ -n "$DATA_DIR" && -d "$DATA_DIR" ]] && rm -rf "$DATA_DIR"
  rm -f "$ENV_FILE"
else
  info "kept $ENV_FILE and ${DATA_DIR:-data dir} (--keep-data)"
fi

say "Uninstalled. The checkout at $ROOT and any backups/ remain — remove them manually."
info "Runner images (ghcr.io/…/runway-runner-*) are kept; prune with \`docker image prune\`."
