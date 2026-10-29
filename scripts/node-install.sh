#!/usr/bin/env sh
# node-install.sh — enroll this machine as a Runway remote Docker node.
#
#   curl -fsSL https://<your-runway>/install-node.sh | \
#     sudo RUNWAY_URL=https://<your-runway> RUNWAY_TOKEN=rnod_… sh
#
# What it does, in order:
#   1. preflight — root, systemd, openssl, curl; Docker installed if absent
#      (RUNWAY_SKIP_DOCKER_INSTALL=1 refuses instead)
#   2. generates a dedicated CA + server cert + client cert under
#      /etc/docker/runway-tls/ (kept on reruns — tokens rotate, certs stay)
#   3. a systemd drop-in puts dockerd on tcp://0.0.0.0:2376 with tlsverify
#      alongside the existing unix socket; daemon.json is left untouched
#   4. restarts dockerd, verifies the TLS port answers
#   5. POSTs the client material + inventory to the control plane; the
#      node lands "waiting" until an admin approves it
#
# Env:  RUNWAY_URL (required), RUNWAY_TOKEN (required, rnod_…),
#       RUNWAY_HOST (override detected address), RUNWAY_PORT (default 2376),
#       RUNWAY_NODE_NAME (default: hostname)

set -eu

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

TLS_DIR=/etc/docker/runway-tls
PORT="${RUNWAY_PORT:-2376}"

# --- preflight --------------------------------------------------------------

[ "$(id -u)" = 0 ] || die "run as root: curl … | sudo RUNWAY_URL=… RUNWAY_TOKEN=rnod_… sh"
[ "$(uname -s)" = Linux ] || die "nodes must run Linux"
command -v systemctl >/dev/null || die "systemd required — no supported service manager found"
command -v openssl  >/dev/null || die "openssl required"
command -v curl     >/dev/null || die "curl required"

[ -n "${RUNWAY_URL:-}" ]   || die "RUNWAY_URL not set — copy the full install line from Settings → Nodes"
[ -n "${RUNWAY_TOKEN:-}" ] || die "RUNWAY_TOKEN not set — copy the full install line from Settings → Nodes"
case "$RUNWAY_TOKEN" in
  rnod_*) [ ${#RUNWAY_TOKEN} -eq 53 ] || die "RUNWAY_TOKEN doesn't look like one — it is rnod_ + 48 characters" ;;
  *) die "RUNWAY_TOKEN doesn't look like one — it starts with rnod_" ;;
esac

# The URL must be Runway, not a proxy or a wrong address (ployz pattern).
health="$(curl -fsS --max-time 10 "$RUNWAY_URL/health" 2>/dev/null || true)"
case "$health" in
  *'"service":"runway"'*) ;;
  *) die "$RUNWAY_URL isn't how Runway answers — a proxy or firewall may have answered instead. Check RUNWAY_URL" ;;
esac

# --- docker ------------------------------------------------------------------

if ! command -v dockerd >/dev/null; then
  [ "${RUNWAY_SKIP_DOCKER_INSTALL:-0}" = 1 ] && \
    die "Docker not installed and RUNWAY_SKIP_DOCKER_INSTALL=1 — install Docker 24+ first"
  say "Installing Docker"
  curl -fsSL https://get.docker.com | sh >/dev/null
  command -v dockerd >/dev/null || die "Docker install did not provide dockerd"
fi
dockerd --version >/dev/null 2>&1 || die "dockerd present but not runnable"

DOCKERD_BIN="$(command -v dockerd)"

# --- address + name ----------------------------------------------------------

if [ -z "${RUNWAY_HOST:-}" ]; then
  RUNWAY_HOST="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' | head -1)"
  [ -n "$RUNWAY_HOST" ] || RUNWAY_HOST="$(hostname -I 2>/dev/null | awk '{print $1}')"
  [ -n "$RUNWAY_HOST" ] || die "cannot determine this machine's address — set RUNWAY_HOST"
fi
# Runway dials the node — the address must be reachable *from the control
# plane*. A NAT/private outbound address works only if Runway shares the LAN.
case "$RUNWAY_HOST" in
  10.*|172.1[6-9].*|172.2[0-9].*|172.3[0-1].*|192.168.*|127.*|169.254.*)
    warn "$RUNWAY_HOST is a private/loopback address — Runway can only reach"
    warn "this node if it shares the LAN or a tunnel. Set RUNWAY_HOST to the"
    warn "public/peer-reachable address otherwise."
    ;;
esac
# SAN entries are typed: IPs go under IP:, names under DNS:.
case "$RUNWAY_HOST" in
  *[!0-9.]*) HOST_SAN="DNS:$RUNWAY_HOST" ;;
  *)         HOST_SAN="IP:$RUNWAY_HOST" ;;
esac
NODE_NAME="${RUNWAY_NODE_NAME:-$(hostname -s 2>/dev/null | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9-' '-' | sed 's/^-*//;s/-*$//')}"
[ -n "$NODE_NAME" ] || NODE_NAME="$RUNWAY_HOST"

# --- certificates ------------------------------------------------------------

mkdir -p "$TLS_DIR"; chmod 700 "$TLS_DIR"
if [ -f "$TLS_DIR/ca.pem" ] && [ -f "$TLS_DIR/server.pem" ] && [ -f "$TLS_DIR/client.pem" ]; then
  info "certificates already present — keeping them"
else
  say "Generating mTLS material in $TLS_DIR"
  # Dedicated CA
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$TLS_DIR/ca-key.pem" -out "$TLS_DIR/ca.pem" -days 3650 \
    -subj "/CN=runway-node-ca" 2>/dev/null
  # Server cert — SAN covers the reachable address + hostname + loopback
  cat > "$TLS_DIR/server-ext.cnf" <<EOF
subjectAltName=$HOST_SAN,DNS:$(hostname -f 2>/dev/null || hostname),IP:127.0.0.1
extendedKeyUsage=serverAuth
EOF
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$TLS_DIR/server-key.pem" -out "$TLS_DIR/server.csr" \
    -subj "/CN=runway-node" 2>/dev/null
  openssl x509 -req -in "$TLS_DIR/server.csr" -CA "$TLS_DIR/ca.pem" \
    -CAkey "$TLS_DIR/ca-key.pem" -CAcreateserial -days 825 \
    -extfile "$TLS_DIR/server-ext.cnf" -out "$TLS_DIR/server.pem" 2>/dev/null
  # Client cert — Runway presents this to dockerd
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$TLS_DIR/client-key.pem" -out "$TLS_DIR/client.csr" \
    -subj "/CN=runway-client" 2>/dev/null
  openssl x509 -req -in "$TLS_DIR/client.csr" -CA "$TLS_DIR/ca.pem" \
    -CAkey "$TLS_DIR/ca-key.pem" -CAcreateserial -days 825 \
    -out "$TLS_DIR/client.pem" 2>/dev/null
  rm -f "$TLS_DIR"/*.csr "$TLS_DIR/server-ext.cnf"
  chmod 600 "$TLS_DIR"/*-key.pem
fi

# --- dockerd on tcp+tls ------------------------------------------------------

say "Configuring dockerd"
DROPIN=/etc/systemd/system/docker.service.d/runway-tls.conf
mkdir -p "$(dirname "$DROPIN")"
cat > "$DROPIN" <<EOF
[Service]
ExecStart=
ExecStart=$DOCKERD_BIN --containerd=/run/containerd/containerd.sock -H unix:///var/run/docker.sock -H tcp://0.0.0.0:$PORT --tlsverify --tlscacert=$TLS_DIR/ca.pem --tlscert=$TLS_DIR/server.pem --tlskey=$TLS_DIR/server-key.pem
EOF
systemctl daemon-reload
systemctl restart docker
sleep 1
systemctl is-active --quiet docker || die "docker failed to start with TLS config — check: journalctl -u docker -n 50"

# Verify the TLS port answers with the client cert.
code="$(curl -sk --cacert "$TLS_DIR/ca.pem" --cert "$TLS_DIR/client.pem" \
  --key "$TLS_DIR/client-key.pem" -o /dev/null -w '%{http_code}' \
  --max-time 10 "https://127.0.0.1:$PORT/version" || true)"
[ "$code" = 200 ] || die "docker TLS port not answering on $PORT (got ${code:-timeout}) — check the daemon and firewall"
info "dockerd answering on tcp://$RUNWAY_HOST:$PORT (mTLS)"

# --- check in ----------------------------------------------------------------

say "Checking in with $RUNWAY_URL"
pem_json() { sed ':a;N;$!ba;s/\\/\\\\/g;s/"/\\"/g;s/\n/\\n/g' "$1"; }
payload=$(printf '{"token":"%s","name":"%s","host":"%s","port":%s,"tls_ca":"%s","tls_cert":"%s","tls_key":"%s","meta":{"hostname":"%s","arch":"%s","docker_version":"%s","cpus":%s,"mem_bytes":%s}}' \
  "$RUNWAY_TOKEN" "$NODE_NAME" "$RUNWAY_HOST" "$PORT" \
  "$(pem_json "$TLS_DIR/ca.pem")" \
  "$(pem_json "$TLS_DIR/client.pem")" \
  "$(pem_json "$TLS_DIR/client-key.pem")" \
  "$(hostname -f 2>/dev/null || hostname)" "$(uname -m)" \
  "$(docker version --format '{{.Server.Version}}' 2>/dev/null || echo unknown)" \
  "$(nproc 2>/dev/null || echo 0)" \
  "$(awk '/MemTotal/{print $2*1024}' /proc/meminfo 2>/dev/null || echo 0)")

out="$(mktemp)"; trap 'rm -f "$out"' EXIT
code="$(curl -sS --max-time 15 -o "$out" -w '%{http_code}' -X POST \
  "$RUNWAY_URL/api/v1/nodes/checkin" \
  -H 'content-type: application/json' -d "$payload" 2>/dev/null)" || \
  die "checkin could not reach $RUNWAY_URL — check connectivity and RUNWAY_URL"
if [ "$code" != 201 ] && [ "$code" != 200 ]; then
  reason="$(sed -n 's/.*"error":"\([^"]*\)".*/\1/p' "$out")"
  case "$reason" in
    *expired*)  die "Token expired. Mint a new one under Settings → Nodes and run the install again." ;;
    *revoked*)  die "Token revoked. Mint a new one under Settings → Nodes and run the install again." ;;
    *invalid*)  die "Runway at $RUNWAY_URL refused this token. Check it was copied whole." ;;
    *)          die "Runway refused this node: ${reason:-$(cat "$out")}" ;;
  esac
fi

printf '\n'
say "Checked in — node is waiting for approval"
info "Enroll it in the dashboard under Settings → Nodes, then assign projects to it."
