# Self-hosting

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | bash
```

Interactive setup: chooses local vs. domain install, asks for the
dashboard hostname (deployments default to `<name>.<that-host>`; extra
domains attach per project later), TLS mode (Let's Encrypt /
Cloudflare Tunnel / external proxy), ports, optional SMTP, and the
owner account. The owner email requires a password (min 8 chars) —
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

Settings → Remote Docker nodes: register `tcp://host:2375` (or `:2376`
for mTLS). **mTLS** — the `mTLS` button generates a per-node CA and
returns a one-time server bundle plus the dockerd flags:

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

## Upgrades

Re-run `install.sh` (or `git pull` + `docker compose up -d --build`).
Migrations are embedded and run at startup; they are numbered and
idempotent — never edit an applied migration.

## Operations

- Deploy logs: `data/logs/<deployment>.log`, 30-day retention.
- Metrics: sampled every 30s per running deployment; purge via
  `config.deployment_retention`.
- Health: `config.health_check` probes running containers; `unhealthy`
  feeds crash notifications + status pages.
- Edge: Traefik file provider under `data/traefik/` — regenerated on
  edge-sensitive config changes; inspect there when routing surprises.
