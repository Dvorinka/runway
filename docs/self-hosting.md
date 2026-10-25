# Self-hosting

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | bash
```

Interactive setup: chooses local vs. domain install, asks for the
dashboard hostname (deployments default to `<name>.<that-host>`; extra
domains attach per project later), TLS mode (Let's Encrypt /
Cloudflare Tunnel / external proxy), ports, optional SMTP, and the
owner account. The owner email requires a password —
skipping the account asks for the registration policy instead:
restricted to an address you name (seeded into the sign-up allowlist),
or open (then an owner account is mandatory, since the first
registrant becomes instance admin). The allowlist stays manageable in
Settings. Then it writes `.env` and brings
up `compose/production.yml` (runway + postgres + traefik). Re-runs offer
to keep an existing `.env`. For unattended installs, `-y` reads the env
vars instead of prompting:

```bash
RUNWAY_DIR=/opt/runway RUNWAY_MODE=domain \
APP_HOSTNAME=apps.example.com DEPLOY_DOMAIN=deploy.example.com \
TLS_MODE=le ACME_EMAIL=you@example.com \
  bash install.sh -y
```

`RUNWAY_VERSION` pins a release tag; `RUNWAY_REF` selects a git ref.
From a checkout, `./install.sh` installs in place. First run: create the
owner account via `runway bootstrap` or the dashboard signup (gated by
the allowlist).

## Required env

| Var | Purpose |
|---|---|
| `APP_HOSTNAME` | dashboard/API host |
| `DEPLOY_DOMAIN` | wildcard base for deployment URLs |
| `DATABASE_URL` | postgres DSN |
| `SECRET_KEY` | JWT/session signing |
| `ENCRYPTION_KEY` | secret encryption at rest |
| `ACME_EMAIL` | Let's Encrypt (direct-IP mode) |

## Sign-in methods

Email + password always works. Optional extras, all off by default and
hidden from the login page until configured (`GET /api/auth/providers`
reports which are on):

- **SMTP** (`SMTP_HOST/PORT/USER/PASSWORD/FROM/TLS`) — enables
  magic-link sign-in, email verification, and team-invite mail.
- **GitHub/Google OAuth** (`GITHUB_OAUTH_CLIENT_ID/SECRET`,
  `GOOGLE_CLIENT_ID/SECRET`) — standalone OAuth apps; register the
  callback as `{scheme}://{APP_HOSTNAME}/api/auth/oauth/{provider}/callback`.
- **OIDC** (`OIDC_*`) — enterprise SSO, discovery-URL driven.
- **TOTP 2FA** — per-user, in Settings; no env needed.

## Networking

Two ways to get traffic in:

- **Direct IP + ACME** — point DNS at the host; Traefik terminates TLS.
- **Cloudflare tunnel** — set `CF_API_TOKEN` + `CF_ACCOUNT_ID` (the
  installer asks for them); the platform provisions the instance tunnel
  and DNS for `APP_HOSTNAME` and `*.DEPLOY_DOMAIN`. Works behind CGNAT —
  cloudflared dials out, so no open ports or public IP needed. Create
  the token at dash.cloudflare.com → My Profile → API Tokens → Custom
  token with `Account — Cloudflare Tunnel: Edit`,
  `Account — Account Settings: Read`, `Zone — DNS: Edit`,
  `Zone — Zone: Read`; resources: your account and the zone covering
  `DEPLOY_DOMAIN`. The account ID is on the zone Overview page sidebar.
  The domain's DNS must already live on Cloudflare.

## Remote Docker nodes

The easy path needs one command on the machine. Settings → Remote Docker
nodes → **Get install command** mints a 24-hour `rnod_…` token and prints:

```bash
curl -fsSL https://<your-runway>/install-node.sh | \
  sudo RUNWAY_URL=https://<your-runway> RUNWAY_TOKEN=rnod_… sh
```

The installer checks preflights (root, systemd, openssl, curl), installs
Docker if absent (`RUNWAY_SKIP_DOCKER_INSTALL=1` refuses instead),
generates a dedicated CA + server + client certs under
`/etc/docker/runway-tls/`, drops a systemd unit that puts dockerd on
`tcp://0.0.0.0:2376 --tlsverify` alongside the unix socket, then POSTs
the client material and machine inventory to `/api/v1/nodes/checkin`.
The node lands as **waiting** — approve it in Settings → Nodes (or
`runway nodes approve <name>`) before deployments can route to it.

Prereqs on the node: Linux + systemd, amd64/arm64, outbound IPv4 to reach
the control plane, and an address Runway can dial back on (`RUNWAY_HOST`
overrides auto-detection; private IPs only work on a shared LAN/tunnel).
Firewalls: inbound `2376/tcp` from the control plane, plus the published
app ports Traefik proxies to (`49152+`).

CLI equivalent: `runway nodes add --command [--expires <hours>]`,
`nodes ls`, `nodes approve|rm <id|name>`, `nodes token ls|rm <id>`.

Manual setup stays available under **Manual setup**: register
`tcp://host:2375` (plaintext — trusted networks only) or `:2376` and use
the `mTLS` action to generate a per-node CA and one-time server bundle:

```
dockerd --tlsverify \
  --tlscacert=/etc/docker/runway/ca.pem \
  --tlscert=/etc/docker/runway/server.pem \
  --tlskey=/etc/docker/runway/server-key.pem \
  -H tcp://0.0.0.0:2376
```

Install on the node, restart dockerd, then **Check**. App containers
publish an allocated host port (`49152+`) that Traefik routes
`http://{node.host}:{port}` to — no extra software on the node.
Upload/static deploys stay local (artifacts are host files).

## Runner images

Builds run in per-runtime `ghcr.io/devpushhq/runner-*` images, pulled
on demand. Private registries, mirrors, or an air-gapped host override
them per slug via `$DATA_DIR/runner-overrides.json`:

```json
{ "runners": { "node-20": { "image": "registry.internal/runner-node-20:1.0.1" },
              "bun-1.3": { "enabled": false } } }
```

`enabled: false` blocks a preset entirely. The file is read per call —
edits take effect without a restart.

## Upgrades

`scripts/update.sh` moves the install to a new ref and rebuilds the
stack — `--ref <tag>` to pin, `--backup` to snapshot first. It works
on both install shapes: git checkouts fetch + checkout; tarball
installs get an archive overlay that preserves `.env` and `data/`.
Migrations are embedded and run at startup; they are numbered and
idempotent — never edit an applied migration.

## Operations

- `scripts/status.sh` — compose state, `/health` reachability, version.
- `scripts/restart.sh` — in-place restart with a health wait.
- `scripts/backup.sh` / `scripts/restore.sh` — Postgres + data dir
  archives under `backups/`; restore replaces the current data dir and
  database after a confirmation prompt (`--yes` to skip).
- Deploy logs: `data/logs/<deployment>.log`, 30-day retention.
- Metrics: sampled every 30s per running deployment; purge via
  `config.deployment_retention`.
- Health: `config.health_check` probes running containers; `unhealthy`
  feeds crash notifications + status pages.
- Edge: Traefik file provider under `data/traefik/` — regenerated on
  edge-sensitive config changes; inspect there when routing surprises.
