#!/usr/bin/env bash
# update.sh — move a Runway install to a new ref and rebuild the stack.
#
#   scripts/update.sh [--ref REF] [--backup] [-y]
#
# Works on both install shapes produced by install.sh:
#   git checkout  — fetch + checkout the ref (branches reset to origin)
#   tarball       — download the ref archive and overlay it on the install
#                   dir (.env and data/ are not part of the archive, so
#                   both are preserved)
# Migrations run automatically when the new container starts.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")
REPO="https://github.com/Dvorinka/runway"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USG'
Usage: scripts/update.sh [--ref REF] [--backup] [-y|--yes] [-h]

Update an install.sh-style Runway deployment to a new git ref and
rebuild the containers.

  --ref REF   branch, tag, or sha to update to (default: main)
  --backup    run scripts/backup.sh before touching anything
  -y, --yes   no prompts (required when stdin is not a tty)
  -h          show this help
USG
  exit 0
}

REF="main"
BACKUP=0
YES=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --ref) REF="${2:?--ref needs a value}"; shift 2 ;;
    --backup) BACKUP=1; shift ;;
    -y|--yes) YES=1; shift ;;
    -h|--help) usage ;;
    *) die "unknown option: $1 (try -h)" ;;
  esac
done

[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon — start it, or re-run with sudo / a user in the docker group"

confirm() {
  [[ "$YES" = 1 ]] && return 0
  [[ -t 0 ]] || die "non-interactive: re-run with -y"
  printf '\033[1;36m?\033[0m %s \033[2m[y/N]\033[0m: ' "$1"
  local a; read -r a
  case "$a" in [Yy]*) return 0 ;; *) die "aborted" ;; esac
}

if ((BACKUP)); then
  say "Backing up first"
  bash "$(dirname "${BASH_SOURCE[0]}")/backup.sh"
fi

if [[ -d "$ROOT/.git" ]]; then
  say "Updating sources to $REF (git)"
  if [[ -n "$(git -C "$ROOT" status --porcelain)" ]]; then
    warn "working tree has local changes — they will be discarded"
    confirm "Discard local changes and update to $REF?"
    git -C "$ROOT" reset --hard -q
  fi
  git -C "$ROOT" fetch -q --tags origin
  git -C "$ROOT" checkout -q "$REF"
  # Tracking a branch: snap it to origin. Tags/shas stay detached.
  if git -C "$ROOT" rev-parse --verify -q "origin/$REF" >/dev/null; then
    git -C "$ROOT" reset --hard -q "origin/$REF"
  fi
else
  say "Updating sources to $REF (tarball overlay)"
  command -v tar >/dev/null || die "required tool not found: tar"
  command -v curl >/dev/null || die "required tool not found: curl"
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' EXIT
  curl -fsSL "$REPO/archive/$REF.tar.gz" -o "$tmp"
  # Overlay: archive has no .env or data/, so both survive.
  tar -xzf "$tmp" --strip-components=1 -C "$ROOT"
fi

say "Rebuilding and restarting"
(cd "$ROOT" && "${COMPOSE[@]}" up -d --build)

say "Waiting for the app"
for _ in $(seq 1 60); do
  (cd "$ROOT" && "${COMPOSE[@]}" exec -T runway true) >/dev/null 2>&1 && break
  sleep 2
done
(cd "$ROOT" && "${COMPOSE[@]}" exec -T runway true) >/dev/null 2>&1 || \
  die "app did not come up — check: docker compose -f compose/production.yml logs runway"

host="$(awk -F= '$1=="APP_HOSTNAME"{sub(/^[^=]*=/,"");v=$0}END{print v}' "$ENV_FILE")"
printf '\n'
printf '\033[1;32m==>\033[0m Runway updated to %s\n' "$REF"
[[ -n "$host" ]] && info "https://$host"
