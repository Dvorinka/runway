#!/usr/bin/env bash
# Runway — interactive install.
#
#   curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | bash
#
# Walks through local-vs-domain setup, writes .env, builds the image,
# starts the stack, and can create the first admin account. Prompts read
# from /dev/tty so `curl | bash` stays interactive; -y/--yes (or
# RUNWAY_YES=1) gives a fully unattended install driven by the env vars
# below.
#
# Env overrides:
#   RUNWAY_DIR       install directory           (default ./runway)
#   RUNWAY_REF       git ref to install          (default main)
#   RUNWAY_VERSION   release tag alias for RUNWAY_REF
#   RUNWAY_MODE      local | domain              (skips the mode prompt)
#   APP_HOSTNAME     dashboard hostname          (default runway.localhost)
#   DEPLOY_DOMAIN    wildcard deploy domain      (default: APP_HOSTNAME)
#   DISABLE_TLS      "true" when a tunnel/proxy terminates TLS upstream
#   ACME_EMAIL       Let's Encrypt email (direct-IP TLS)
#   CF_API_TOKEN     Cloudflare API token (managed tunnel)
#   CF_ACCOUNT_ID    Cloudflare account id (managed tunnel)
#   HTTP_PORT        published http port         (default 80)
#   HTTPS_PORT       published https port        (default 443)
#   SMTP_*           optional mail (magic links, invites)
#   BOOTSTRAP_EMAIL  create the owner account after start (password required)
#   BOOTSTRAP_PASSWORD  owner password (required with email)
#   ALLOW_REGISTRATION  open | restricted (default restricted; open with
#                      no owner account is refused — first registrant is admin)
#   ALLOWLIST_SEED_EMAIL  address allowed to register when skipping owner
#                      creation (default: the owner email)
#
# From a git checkout, ./install.sh installs in place (no download).

set -euo pipefail

REPO="https://github.com/Dvorinka/runway"
REF="${RUNWAY_VERSION:-${RUNWAY_REF:-main}}"
DIR="${RUNWAY_DIR:-./runway}"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"; }

usage() {
  cat <<'USG'
Usage: install.sh [-y|--yes] [-h|--help]

Interactive installer for Runway. With -y it never prompts and uses the
RUNWAY_*/APP_HOSTNAME/DEPLOY_DOMAIN/... env vars (see script header).

  -y, --yes    non-interactive; accept defaults / env overrides
  -h, --help   show this help
USG
  exit 0
}

YES="${RUNWAY_YES:-0}"
for arg in "$@"; do
  case "$arg" in
    -y|--yes) YES=1 ;;
    -h|--help) usage ;;
    *) die "unknown option: $arg (try --help)" ;;
  esac
done

# Interactive when a controlling terminal is reachable — prompts read
# /dev/tty so `curl | bash` still works.
interactive=0
if [ "$YES" != 1 ] && [ -z "${CI:-}" ] && { [ -t 0 ] || (: </dev/tty) 2>/dev/null; }; then
  interactive=1
fi

ask() { # ask <prompt> [default] -> REPLY
  local prompt="$1" def="${2:-}"
  if [ -n "$def" ]; then
    printf '\033[1;36m?\033[0m %s \033[2m[%s]\033[0m: ' "$prompt" "$def"
  else
    printf '\033[1;36m?\033[0m %s: ' "$prompt"
  fi
  IFS= read -r REPLY </dev/tty || REPLY=""
  REPLY="${REPLY:-$def}"
}

ask_secret() { # ask_secret <prompt> -> REPLY (dots, value never echoed)
  printf '\033[1;36m?\033[0m %s: ' "$1"
  REPLY=""
  local ch
  while IFS= read -rsn1 ch </dev/tty 2>/dev/null; do
    if [ -z "$ch" ]; then break; fi # Enter
    case "$ch" in
      $'\177'|$'\b') # backspace
        if [ -n "$REPLY" ]; then REPLY="${REPLY%?}"; printf '\b \b'; fi ;;
      *) REPLY+="$ch"; printf '•' ;;
    esac
  done
  printf '\n'
  if [ -n "$REPLY" ]; then info "(${#REPLY} characters entered)"; fi
}

confirm() { # confirm <prompt> [y|n]
  local prompt="$1" def="${2:-y}" a
  if [ "$def" = y ]; then
    printf '\033[1;36m?\033[0m %s \033[2m[Y/n]\033[0m: ' "$prompt"
  else
    printf '\033[1;36m?\033[0m %s \033[2m[y/N]\033[0m: ' "$prompt"
  fi
  IFS= read -r a </dev/tty || a=""
  a="${a:-$def}"
  case "$a" in [Yy]*) return 0 ;; *) return 1 ;; esac
}

valid_domain() { # crude host check: labels of [a-z0-9-] separated by dots
  case "$1" in
    *[!a-z0-9.-]* | .* | *. | *..*) return 1 ;;
    *) [ -n "$1" ] ;;
  esac
}

sanitize_domain() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's#^https?://##; s#/.*$##; s#^\.+##; s#\.+$##'
}

CF_API="https://api.cloudflare.com/client/v4"

cf_token_ok() { # cf_token_ok <token>
  curl -fsSL --max-time 10 -H "Authorization: Bearer $1" \
    "$CF_API/user/tokens/verify" 2>/dev/null | grep -q '"success":true'
}

cf_accounts() { # cf_accounts <token> — prints "id|name" lines
  curl -fsSL --max-time 10 -H "Authorization: Bearer $1" \
    "$CF_API/accounts?per_page=50" 2>/dev/null \
    | tr '{' '\n' \
    | sed -nE 's/.*"id":"([a-f0-9]{32})".*"name":"([^"]+)".*/\1|\2/p'
}

cf_zones() { # cf_zones <token> — prints zone names
  curl -fsSL --max-time 10 -H "Authorization: Bearer $1" \
    "$CF_API/zones?per_page=50" 2>/dev/null \
    | tr '{' '\n' \
    | sed -nE 's/.*"name":"([^"]+)","status":"[a-z]+".*/\1/p'
}

cf_zone_covers() { # cf_zone_covers <hostname> — against global `zones`
  local h="$1" z
  for z in ${zones[@]+"${zones[@]}"}; do
    [ -z "$z" ] && continue
    if [ "$h" = "$z" ] || [ "${h##*."$z"}" != "$h" ]; then return 0; fi
  done
  return 1
}

need curl
need openssl
need docker
docker compose version >/dev/null 2>&1 || die "docker compose plugin not found"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon — start it, or re-run with sudo / a user in the docker group"

# In-place install: already inside a checkout.
if [ -f Cargo.toml ] && [ -f compose/production.yml ]; then
  DIR="."
else
  if [ ! -f "$DIR/compose/production.yml" ]; then
    say "Downloading runway@$REF"
    mkdir -p "$DIR"
    need tar
    tmp="$(mktemp)"
    curl -fsSL "$REPO/archive/$REF.tar.gz" -o "$tmp"
    tar -xzf "$tmp" --strip-components=1 -C "$DIR"
    rm -f "$tmp"
  else
    say "Existing install found at $DIR — reusing sources"
  fi
fi

ENV_FILE="$DIR/.env"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

write_env=1
if [ -f "$ENV_FILE" ]; then
  if [ "$interactive" = 1 ]; then
    if confirm "Existing $ENV_FILE found — keep it?" y; then
      write_env=0
    else
      bak="$ENV_FILE.bak.$(date +%Y%m%d%H%M%S)"
      cp "$ENV_FILE" "$bak"
      info "Old .env backed up to $bak"
    fi
  else
    say ".env exists — keeping it"
    write_env=0
  fi
fi

if [ "$write_env" = 1 ]; then

  if [ "$interactive" = 1 ]; then
    printf '\n'
    say "Where will this instance run?"
    printf '    1) Local   this machine only — http on *.localhost\n'
    printf '    2) Domain  public domain — TLS via Let'"'"'s Encrypt or Cloudflare Tunnel\n'
    ask "Choose" "${RUNWAY_MODE:-1}"
    MODE="$REPLY"
  else
    MODE="${RUNWAY_MODE:-}"
    # Backwards compat: pre-set hostnames imply a domain install.
    [ -z "$MODE" ] && [ -n "${APP_HOSTNAME:-}${DEPLOY_DOMAIN:-}" ] && MODE=domain
    MODE="${MODE:-local}"
  fi
  case "$MODE" in 1|local) MODE=local ;; 2|domain) MODE=domain ;; *)
    die "invalid mode: $MODE (expected local|domain)" ;; esac

  SERVER_IP=""

  if [ "$MODE" = local ]; then
    APP_HOSTNAME="${APP_HOSTNAME:-runway.localhost}"
    DEPLOY_DOMAIN="${DEPLOY_DOMAIN:-deploy.localhost}"
    DISABLE_TLS=true
    TLS_LABEL="http (local)"
    ACME_EMAIL=""
    CF_API_TOKEN="${CF_API_TOKEN:-}"
    CF_ACCOUNT_ID="${CF_ACCOUNT_ID:-}"
    HTTP_PORT="${HTTP_PORT:-80}"
    HTTPS_PORT="${HTTPS_PORT:-443}"

    if [ "$interactive" = 1 ]; then
      ask "HTTP port" "$HTTP_PORT"; HTTP_PORT="$REPLY"
    fi
  else
    # Domain mode
    if [ "$interactive" = 1 ]; then
      printf '\n'
      ask "Dashboard hostname (e.g. runway.example.com)" "${APP_HOSTNAME:-}"
      APP_HOSTNAME="$(sanitize_domain "$REPLY")"
      valid_domain "$APP_HOSTNAME" || die "invalid hostname: $REPLY"
      # Default wildcard base for deployment URLs. Extra domains can be
      # attached per project later (dashboard + API), so no prompt here.
      DEPLOY_DOMAIN="$(sanitize_domain "${DEPLOY_DOMAIN:-$APP_HOSTNAME}")"
      valid_domain "$DEPLOY_DOMAIN" || die "invalid domain: $DEPLOY_DOMAIN"
      info "Deployments get <name>.$DEPLOY_DOMAIN URLs."

      printf '\n'
      say "How does traffic reach this server?"
      info "No public IP (CGNAT, NAT, home connection)? Pick the tunnel —"
      info "Runway provisions it, creates the DNS records, and runs"
      info "cloudflared itself. No open ports needed."
      printf '    1) Cloudflare Tunnel  no public IP required — recommended for CGNAT/home\n'
      printf '    2) Direct public IP   DNS A records point here, Let'"'"'s Encrypt TLS\n'
      printf '    3) External proxy     another edge terminates TLS in front\n'
      ask "Choose" "${TLS_MODE:-1}"
      TLS_MODE="$REPLY"
    else
      APP_HOSTNAME="${APP_HOSTNAME:?set APP_HOSTNAME for a domain install}"
      DEPLOY_DOMAIN="${DEPLOY_DOMAIN:-$APP_HOSTNAME}"
      TLS_MODE="${TLS_MODE:-}"
      if [ -z "$TLS_MODE" ]; then
        if [ "${DISABLE_TLS:-}" = false ]; then TLS_MODE=le
        elif [ -n "${CF_API_TOKEN:-}" ]; then TLS_MODE=tunnel
        else TLS_MODE=proxy; fi
      fi
    fi

    case "$TLS_MODE" in
      1|tunnel|cf|cloudflare)
        DISABLE_TLS=true
        TLS_LABEL="cloudflare tunnel"
        ACME_EMAIL=""
        if [ "$interactive" = 1 ]; then
          cat <<EOF

    The domain's DNS must be managed by Cloudflare. Create the token:

      https://dash.cloudflare.com/profile/api-tokens
      → Create Token → Custom token, with these permissions:

           Account — Cloudflare Tunnel — Edit
           Account — Account Settings  — Read
           Zone    — DNS               — Edit
           Zone    — Zone              — Read

      Resources: your account + the zone covering $DEPLOY_DOMAIN.

    On first start Runway creates the "runway-instance" tunnel, points
    $APP_HOSTNAME and *.$DEPLOY_DOMAIN at it, and launches a
    cloudflared container. Nothing else to set up.

EOF
          ask_secret "Cloudflare API token (empty to configure later)"
          CF_API_TOKEN="${REPLY:-${CF_API_TOKEN:-}}"
          if [ -n "$CF_API_TOKEN" ]; then
            say "Checking token"
            if cf_token_ok "$CF_API_TOKEN"; then
              info "token valid"
            else
              warn "token rejected by Cloudflare — check permissions; continuing anyway"
            fi
            mapfile -t accts < <(cf_accounts "$CF_API_TOKEN")
            if [ "${#accts[@]}" -eq 1 ]; then
              CF_ACCOUNT_ID="${accts[0]%%|*}"
              info "account: ${accts[0]#*|} ($CF_ACCOUNT_ID)"
            elif [ "${#accts[@]}" -gt 1 ]; then
              say "Token can see multiple accounts:"
              for i in "${!accts[@]}"; do
                info "$((i + 1))) ${accts[i]#*|} (${accts[i]%%|*})"
              done
              ask "Choose (or paste an account ID)" "${CF_ACCOUNT_ID:-1}"
              if [[ "$REPLY" =~ ^[0-9]+$ ]] && [ "$REPLY" -le "${#accts[@]}" ] && [ "$REPLY" -ge 1 ]; then
                CF_ACCOUNT_ID="${accts[$((REPLY - 1))]%%|*}"
              else
                CF_ACCOUNT_ID="$REPLY"
              fi
            else
              ask "Cloudflare account ID (zone Overview page, right sidebar)" "${CF_ACCOUNT_ID:-}"
              CF_ACCOUNT_ID="$REPLY"
            fi
            mapfile -t zones < <(cf_zones "$CF_API_TOKEN")
            if [ "${#zones[@]}" -gt 0 ]; then
              missing=""
              cf_zone_covers "$APP_HOSTNAME" || missing=" $APP_HOSTNAME"
              cf_zone_covers "$DEPLOY_DOMAIN" || missing="$missing $DEPLOY_DOMAIN"
              [ -n "$missing" ] && warn "no Cloudflare zone covers:${missing} — add the domain to this account first"
            fi
          else
            CF_ACCOUNT_ID="${CF_ACCOUNT_ID:-}"
            info "Skipped — set CF_API_TOKEN + CF_ACCOUNT_ID in $ENV_FILE later."
          fi
        else
          CF_API_TOKEN="${CF_API_TOKEN:-}"; CF_ACCOUNT_ID="${CF_ACCOUNT_ID:-}"
        fi
        ;;
      2|le|acme|letsencrypt)
        DISABLE_TLS=false
        TLS_LABEL="let's encrypt"
        CF_API_TOKEN=""; CF_ACCOUNT_ID=""
        if [ "$interactive" = 1 ]; then
          ask "Let's Encrypt email" "${ACME_EMAIL:-}"
          ACME_EMAIL="$REPLY"
          case "$ACME_EMAIL" in *@*) : ;; *) die "invalid email: $ACME_EMAIL" ;; esac
        else
          ACME_EMAIL="${ACME_EMAIL:-}"
        fi
        ;;
      3|proxy|external)
        DISABLE_TLS=true
        TLS_LABEL="external proxy"
        ACME_EMAIL=""; CF_API_TOKEN=""; CF_ACCOUNT_ID=""
        ;;
      *) die "invalid TLS mode: $TLS_MODE" ;;
    esac

    HTTP_PORT="${HTTP_PORT:-80}"
    HTTPS_PORT="${HTTPS_PORT:-443}"
    if [ "$interactive" = 1 ] && [ "$DISABLE_TLS" = false ]; then
      ask "HTTP port" "$HTTP_PORT"; HTTP_PORT="$REPLY"
      ask "HTTPS port" "$HTTPS_PORT"; HTTPS_PORT="$REPLY"
    fi

    SERVER_IP="$(curl -4 -fsSL --max-time 3 https://api.ipify.org 2>/dev/null || true)"
  fi

  # Port sanity check — warn only, Docker reports the real failure.
  if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ":${HTTP_PORT} "; then
    warn "something already listens on port $HTTP_PORT"
  fi

  # Optional SMTP — enables magic-link sign-in, verification, invites.
  SMTP_HOST="${SMTP_HOST:-}"; SMTP_PORT="${SMTP_PORT:-587}"
  SMTP_USER="${SMTP_USER:-}"; SMTP_PASSWORD="${SMTP_PASSWORD:-}"
  SMTP_FROM="${SMTP_FROM:-}"; SMTP_TLS="${SMTP_TLS:-true}"
  if [ "$interactive" = 1 ]; then
    printf '\n'
    if confirm "Configure SMTP for email sign-in and invites?" n; then
      ask "SMTP host" "$SMTP_HOST"; SMTP_HOST="$REPLY"
      ask "SMTP port" "$SMTP_PORT"; SMTP_PORT="$REPLY"
      ask "SMTP user (empty if none)" "$SMTP_USER"; SMTP_USER="$REPLY"
      ask_secret "SMTP password (empty if none)"
      SMTP_PASSWORD="$REPLY"
      ask "From address" "${SMTP_FROM:-runway@$APP_HOSTNAME}"; SMTP_FROM="$REPLY"
    fi
  fi

  BOOTSTRAP_EMAIL="${BOOTSTRAP_EMAIL:-}"
  BOOTSTRAP_PASSWORD="${BOOTSTRAP_PASSWORD:-}"
  ALLOWLIST_SEED_EMAIL="${ALLOWLIST_SEED_EMAIL:-}"
  REGISTRATION_POLICY="${ALLOW_REGISTRATION:-restricted}"
  if [ "$interactive" = 1 ]; then
    printf '\n'
    require_email=0
    while true; do
      if [ "$require_email" = 1 ]; then
        ask "Owner account email (required)" "$BOOTSTRAP_EMAIL"
      else
        ask "Owner account email (empty to skip — first registered user becomes admin)" "$BOOTSTRAP_EMAIL"
      fi
      BOOTSTRAP_EMAIL="$REPLY"
      if [ -z "$BOOTSTRAP_EMAIL" ]; then
        if [ "$require_email" = 1 ]; then
          warn "an owner account is required here"
          continue
        fi
        # No account: the first registrant becomes superadmin (id 1), so
        # registration must be restricted to an address they control —
        # unless they go back and create the owner account.
        printf '\n'
        say "No owner account — the first user to register becomes the instance admin."
        printf '    1) Restricted  only an address you name can register (recommended)\n'
        printf '    2) Open        anyone can register (requires an owner account)\n'
        ask "Choose" "1"
        case "$REPLY" in
          1|restricted) REGISTRATION_POLICY=restricted ;;
          2|open) REGISTRATION_POLICY=open ;;
          *) warn "invalid choice: $REPLY"; continue ;;
        esac
        if [ "$REGISTRATION_POLICY" = restricted ]; then
          ask "Owner email (only this address will be able to register)" "$ALLOWLIST_SEED_EMAIL"
          ALLOWLIST_SEED_EMAIL="$REPLY"
          case "$ALLOWLIST_SEED_EMAIL" in *@*) : ;; *)
            warn "invalid email: $ALLOWLIST_SEED_EMAIL"; continue ;; esac
          break
        fi
        warn "Open registration needs an owner so you keep the admin seat."
        require_email=1
        continue
      fi
      case "$BOOTSTRAP_EMAIL" in *@*) : ;; *)
        warn "invalid email: $BOOTSTRAP_EMAIL"; continue ;; esac
      # Password is mandatory with an account — a passwordless owner
      # cannot sign in (no SMTP magic link) and blocks later signup.
      while true; do
        ask_secret "Owner password (required)"
        BOOTSTRAP_PASSWORD="$REPLY"
        if [ -n "$BOOTSTRAP_PASSWORD" ]; then break; fi
        warn "password must not be empty"
      done
      break
    done
  else
    # Unattended: same rules, no prompts.
    if [ -n "$BOOTSTRAP_EMAIL" ]; then
      case "$BOOTSTRAP_EMAIL" in *@*) : ;; *) die "invalid email: $BOOTSTRAP_EMAIL" ;; esac
      if [ -z "$BOOTSTRAP_PASSWORD" ]; then
        die "BOOTSTRAP_PASSWORD is required with BOOTSTRAP_EMAIL"
      fi
      # Restricted (the default) also closes sign-up to the owner.
      ALLOWLIST_SEED_EMAIL="${ALLOWLIST_SEED_EMAIL:-$BOOTSTRAP_EMAIL}"
    else
      case "$REGISTRATION_POLICY" in restricted|open) : ;; *)
        die "invalid ALLOW_REGISTRATION: $REGISTRATION_POLICY (open|restricted)" ;; esac
      if [ "$REGISTRATION_POLICY" = open ]; then
        die "ALLOW_REGISTRATION=open with no BOOTSTRAP_EMAIL hands admin to the first stranger — set both, or use restricted"
      fi
      case "$ALLOWLIST_SEED_EMAIL" in *@*) : ;; *)
        die "ALLOWLIST_SEED_EMAIL is required when skipping BOOTSTRAP_EMAIL (nobody could register otherwise)" ;; esac
    fi
  fi

  if [ "$interactive" = 1 ]; then
    printf '\n'
    say "Summary"
    info "mode:           $MODE"
    info "dashboard:      $APP_HOSTNAME"
    info "deploy domain:  *.$DEPLOY_DOMAIN"
    info "tls:            ${TLS_LABEL:-http}"
    info "ports:          http=$HTTP_PORT https=$HTTPS_PORT"
    [ -n "$SMTP_HOST" ] && info "smtp:           $SMTP_HOST:$SMTP_PORT"
    if [ -n "$BOOTSTRAP_EMAIL" ]; then
      info "owner:          $BOOTSTRAP_EMAIL"
    else
      info "owner:          (skipped — first registrant becomes admin)"
    fi
    info "registration:   $REGISTRATION_POLICY$([ "$REGISTRATION_POLICY" = restricted ] && [ -n "$ALLOWLIST_SEED_EMAIL" ] && echo " ($ALLOWLIST_SEED_EMAIL)")"
    printf '\n'
    confirm "Install with these settings?" y || die "aborted"
  fi

  say "Generating .env"
  cat > "$ENV_FILE" <<EOF
# Generated by install.sh — edit as needed, never commit.
APP_HOSTNAME=$APP_HOSTNAME
DEPLOY_DOMAIN=$DEPLOY_DOMAIN
SECRET_KEY=$(openssl rand -hex 32)
ENCRYPTION_KEY=$(openssl rand -hex 32)
POSTGRES_PASSWORD=$(openssl rand -hex 24)
HTTP_PORT=$HTTP_PORT
HTTPS_PORT=$HTTPS_PORT
# TLS termination: "true" when behind a tunnel/reverse proxy or local,
# "false" for direct-IP + ACME (needs ACME_EMAIL and public 443).
DISABLE_TLS=$DISABLE_TLS
ACME_EMAIL=$ACME_EMAIL
SERVER_IP=$SERVER_IP
# Instance Cloudflare Tunnel (CGNAT path) — set both to auto-manage.
CF_API_TOKEN=$CF_API_TOKEN
CF_ACCOUNT_ID=$CF_ACCOUNT_ID
# SMTP — magic-link sign-in, verification, team invites.
SMTP_HOST=$SMTP_HOST
SMTP_PORT=$SMTP_PORT
SMTP_USER=$SMTP_USER
SMTP_PASSWORD=$SMTP_PASSWORD
SMTP_FROM=$SMTP_FROM
SMTP_TLS=$SMTP_TLS
# GitHub App / OAuth (optional — the dashboard can register a private app
# via Settings → GitHub App instead).
GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID:-}
GITHUB_CLIENT_SECRET=${GITHUB_CLIENT_SECRET:-}
GITHUB_APP_ID=${GITHUB_APP_ID:-}
GITHUB_APP_NAME=${GITHUB_APP_NAME:-}
GITHUB_APP_PRIVATE_KEY=${GITHUB_APP_PRIVATE_KEY:-}
GITHUB_APP_WEBHOOK_SECRET=${GITHUB_APP_WEBHOOK_SECRET:-}
EOF
  chmod 600 "$ENV_FILE"
fi

# ---------------------------------------------------------------------------
# Build & start
# ---------------------------------------------------------------------------

say "Building and starting the stack"
# NOTE: run compose from inside $DIR. Using
# `--project-directory "$DIR"` from outside re-bases the relative
# `build: ..` context in compose/production.yml to the parent of $DIR,
# so the build looks for ./Dockerfile in the wrong place and fails with
# "failed to read dockerfile: open Dockerfile: no such file or directory".
(cd "$DIR" && docker compose --env-file .env \
  -f compose/production.yml up -d --build)

HOST="$(grep -E '^APP_HOSTNAME=' "$ENV_FILE" | cut -d= -f2-)"
PORT="$(grep -E '^HTTP_PORT=' "$ENV_FILE" | cut -d= -f2-)"
TLS="$(grep -E '^DISABLE_TLS=' "$ENV_FILE" | cut -d= -f2-)"
SCHEME=http
if [ "$TLS" = false ]; then
  SCHEME=https
  PORT="$(grep -E '^HTTPS_PORT=' "$ENV_FILE" | cut -d= -f2-)"
fi
SHOW_PORT="$PORT"
{ [ "$SCHEME" = http ] && [ "$PORT" = 80 ]; } || [ "$PORT" = 443 ] && SHOW_PORT=""

DIR_ABS="$(cd "$DIR" && pwd)"
COMPOSE_CD="cd $DIR_ABS && docker compose --env-file .env -f compose/production.yml"

# Optional first-run bootstrap. The password is always set when an
# account is created (enforced above); a restricted policy seeds the
# sign-up allowlist so registration stays closed (manage in Settings).
if [ -n "${BOOTSTRAP_EMAIL:-}" ]; then
  say "Waiting for the runway container"
  for _ in $(seq 1 45); do
    (cd "$DIR" && docker compose --env-file .env -f compose/production.yml exec -T runway true) >/dev/null 2>&1 && break
    sleep 2
  done
  say "Creating owner account ($BOOTSTRAP_EMAIL)"
  (cd "$DIR" && docker compose --env-file .env -f compose/production.yml exec -T runway runway bootstrap --email "$BOOTSTRAP_EMAIL" --password "$BOOTSTRAP_PASSWORD") || \
    warn "bootstrap failed — retry later: $COMPOSE_CD exec runway runway bootstrap --email $BOOTSTRAP_EMAIL --password '<new-password>'"
fi
# A created account under a restricted policy also closes sign-up to
# the owner (manage the allowlist in Settings to open up later).
if [ -n "${BOOTSTRAP_EMAIL:-}" ] && [ "$REGISTRATION_POLICY" = restricted ] && [ -z "${ALLOWLIST_SEED_EMAIL:-}" ]; then
  ALLOWLIST_SEED_EMAIL="$BOOTSTRAP_EMAIL"
fi
if [ "$REGISTRATION_POLICY" = restricted ] && [ -n "${ALLOWLIST_SEED_EMAIL:-}" ]; then
  say "Waiting for the runway container"
  for _ in $(seq 1 45); do
    (cd "$DIR" && docker compose --env-file .env -f compose/production.yml exec -T runway true) >/dev/null 2>&1 && break
    sleep 2
  done
  say "Restricting registration to $ALLOWLIST_SEED_EMAIL"
  (cd "$DIR" && docker compose --env-file .env -f compose/production.yml exec -T runway runway allowlist-add --email "$ALLOWLIST_SEED_EMAIL") || \
    warn "allowlist seed failed — retry later: $COMPOSE_CD exec runway runway allowlist-add --email $ALLOWLIST_SEED_EMAIL"
fi

cat <<EOF

Runway is up.

  Dashboard:  $SCHEME://${HOST}${SHOW_PORT:+:$SHOW_PORT}
  Bootstrap:  $COMPOSE_CD exec runway runway bootstrap --email you@example.com
  Logs:       $COMPOSE_CD logs -f
EOF

DEPLOY="$(grep -E '^DEPLOY_DOMAIN=' "$ENV_FILE" | cut -d= -f2-)"
CF="$(grep -E '^CF_API_TOKEN=' "$ENV_FILE" | cut -d= -f2-)"
SERVER_IP="$(grep -E '^SERVER_IP=' "$ENV_FILE" | cut -d= -f2-)"

case "$HOST" in
  *.localhost|localhost)
    cat <<EOF

*.localhost resolves to 127.0.0.1 on most systems — if deploy URLs don't
resolve, add "$HOST" and "*.$DEPLOY" to /etc/hosts.
EOF
    ;;
  *)
    if [ -n "$CF" ]; then
      cat <<EOF

The Cloudflare tunnel is pre-configured — on first start Runway creates
the tunnel, points $HOST and *.$DEPLOY at it, and launches
the cloudflared-instance container. Watch it connect with:

  $COMPOSE_CD logs -f runway
  docker logs -f cloudflared-instance
EOF
    else
      {
        echo ""
        echo "DNS — point these at this server${SERVER_IP:+ ($SERVER_IP)}:"
        echo "  A      $HOST"
        [ "$DEPLOY" != "$HOST" ] && echo "  A      $DEPLOY"
        echo "  A      *.$DEPLOY"
      }
    fi
    ;;
esac
