# User docs — shipped behavior

What Runway does from the user's seat. Describes what shipped, not what
is planned; update in the same PR as the behavior change.

- **Sign-in** — email + password; first registered user becomes instance
  admin. No mail server or OAuth app required.
- **Projects** — create from a repo or preset; each project gets a
  `*.DEPLOY_DOMAIN` alias and one production alias.
- **Deploys** — `runway link` in a repo, then `runway deploy --follow`;
  lifecycle `prepare → deploy → finalize → completed`. Rollback re-points
  the alias instantly (`runway rollback`).
- **Env vars** — `runway env set/pull`; stored encrypted, decrypted only
  at deploy/runtime and for `env pull`.
- **Domains** — custom domains verify then route; `assign-cf` binds one
  to a team Cloudflare tunnel for hosts without a public IP.
- **Logs & stats** — per-deployment build/runtime logs, `--project`
  merged tail, live CPU/mem/net snapshot.
- **Teams** — membership boundary for projects, tunnels, API keys.
- **Health checks** — HTTP probes write `unhealthy` observed status,
  notify, and auto-requeue — evidence, not guesses.
- **Notifications** — deploy lifecycle and crash alerts; unread badge in
  the header.
- **Analytics** — first-party privacy-preserving web analytics (daily
  HMAC rotation, no IPs) plus optional RUM speed insights.
