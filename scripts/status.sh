#!/usr/bin/env bash
# status.sh — is the Runway stack up, and is the app answering?
#
#   scripts/status.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env"
COMPOSE=(docker compose --env-file "$ENV_FILE" -f "$ROOT/compose/production.yml")

die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ -f "$ENV_FILE" ]] || die "no .env at $ENV_FILE — run from an install.sh layout"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon"

env_val() { awk -F= -v k="$1" '$1==k {sub(/^[^=]*=/,""); v=$0} END{print v}' "$ENV_FILE"; }

printf '\n'
(cd "$ROOT" && "${COMPOSE[@]}" ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}" 2>/dev/null) || true
printf '\n'

host="$(env_val APP_HOSTNAME)"
scheme="https"
[[ "$(env_val DISABLE_TLS)" = "true" ]] && scheme="http"
if [[ -n "$host" ]]; then
  code="$(curl -sk -o /dev/null -w '%{http_code}' --max-time 5 "$scheme://$host/health" || true)"
  if [[ "$code" = 200 ]]; then
    printf "Health: \033[1;32mok\033[0m  (%s://%s/health)\n" "$scheme" "$host"
  else
    printf "Health: \033[1;33munreachable\033[0m (%s://%s/health → %s)\n" "$scheme" "$host" "${code:-timeout}"
  fi
fi
ver="$(cd "$ROOT" && "${COMPOSE[@]}" exec -T runway runway --version 2>/dev/null | tr -d '\r' || true)"
[[ -n "$ver" ]] && printf 'Version: %s\n' "$ver"
