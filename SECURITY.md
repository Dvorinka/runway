# Security Policy

## Reporting a vulnerability

**Do not open a public issue.** Report privately via
[GitHub Security Advisories](https://github.com/Dvorinka/runway/security/advisories/new)
or email **contact.dvorak@gmail.com** with:

- Affected component and version/commit
- Reproduction steps or proof of concept
- Impact assessment

You will get an acknowledgement within a few days and credit in the
release notes unless you prefer otherwise.

## Scope

Anything in the Runway codebase: auth/session handling, deploy
pipeline, edge/routing layer, secrets at rest, webhook verification,
API key and token flows, MCP surface, remote-node mTLS.

Dependency vulnerabilities are best reported to the upstream crate and
mentioned here for tracking.

## Self-hosting hardening checklist

- Set a strong random `SECRET_KEY` and `ENCRYPTION_KEY` — `install.sh`
  generates these; never reuse values between installs.
- Keep `APP_HOSTNAME` and `DEPLOY_DOMAIN` on real TLS. Direct-IP mode
  uses Let's Encrypt; tunnel mode is TLS-terminated at Cloudflare.
- Restrict the sign-up allowlist in Settings after creating your admin
  account — the first registered user becomes admin.
- Rotate `ak_` API keys and `dp_` deploy-hook tokens like passwords;
  they are stored hashed, so lost keys must be regenerated, not
  recovered.
- Webhook secrets (`GITEA_WEBHOOK_SECRET`, `GITLAB_WEBHOOK_SECRET`,
  `BITBUCKET_WEBHOOK_SECRET`, `GITHUB_APP_WEBHOOK_SECRET`) should be set
  on every connected provider — unsigned payloads are rejected.
- Remote Docker nodes: use the mTLS provisioning endpoint
  (`POST /admin/nodes/{id}/tls`) rather than exposing an unauthenticated
  `tcp://` socket.
- The dashboard binds whatever port you publish; do not expose Postgres
  or the Docker socket to the network.
