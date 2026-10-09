# Agents: CLI, API keys, MCP

Everything the dashboard does is reachable headless — one REST API,
three front doors.

## CLI

```bash
runway login                      # browser or API-key auth
runway link                       # link cwd → project (.runway/project.json)
runway deploy --follow            # upload source, stream build logs
runway logs --follow              # runtime logs (deployment)
runway logs --project             # merged tail across recent deployments
runway stats                      # live CPU/mem/net snapshot
runway env set KEY=value          # env vars
runway env pull                   # .env.local from project (decrypted)
runway domains add app.example.com
runway domains assign-cf app.example.com
runway rollback                   # instant alias re-point to previous deploy
runway open                       # current deployment URL
```

Auth config: `~/.config/runway`. Install via release tarball, the npm
wrapper (`npm i -g @tdvorakdev/runway`), or Homebrew
(`homebrew/runway.rb`).

## API keys

Settings → API keys mint `ak_` tokens. Use as
`Authorization: Bearer ak_…` against `/api/v1/*`. Deploy tokens
(`dp_`) authenticate push-to-deploy CI without full account access.

## MCP

`POST /api/mcp` speaks JSON-RPC 2.0 (`initialize` / `tools/list` /
`tools/call`). Tool surface:

`list_projects`, `get_project`, `list_deployments`, `get_deployment`,
`get_deployment_logs`, `deployment_stats`, `deploy_project`,
`cancel_deployment`, `redeploy_deployment`, `rollback_environment`,
`list_env`, `set_env`, `list_domains`, `ping`.

Write tools (`deploy_project`, `set_env`, `rollback_environment`, …)
respect the same RBAC gates as the REST surface — issue the key to an
account with the minimum role the automation needs.

## Webhooks

Per-project `config.webhook_url` receives deploy lifecycle events
(`deployment.succeeded`, `deployment.failed`, `deployment.crashed`).
Project → Settings → Webhooks manages additional endpoints.

## Provider webhooks

GitHub (app manifest or manual), GitLab, Gitea, and Bitbucket push to
`/api/{provider}/webhook`. Bitbucket payloads are verified by resolving
the real branch head through the connection API — the payload sha is
never trusted.
