# `runway.json` and project config

Two config surfaces:

- **`runway.json`** — committed to the repo; overrides build-time
  behavior inside the runner container.
- **Project `config`** — server-side project settings, patched via
  `PATCH /api/v1/projects/{id}` or the dashboard Settings tab.

## Repo `runway.json`

```json
{
  "build_command": "pnpm run build",
  "start_command": "node server.js",
  "pre_deploy_command": "npx prisma migrate deploy",
  "output_directory": "dist",
  "redirects": [{ "source": "/old", "destination": "/new", "status_code": 308 }],
  "rewrites": [{ "source": "/api/:path*", "destination": "https://api.example.com/:path*" }],
  "headers": [{ "source": "/assets/:path*", "headers": { "cache-control": "public, immutable" } }]
}
```

`redirects`/`rewrites`/`headers` follow the `vercel.json` shape and
translate into the static server's config on static deploys.

## Project `config` keys

| Key | Type | Effect |
|---|---|---|
| `preset` | string | force preset slug |
| `runner` | string | runner image slug |
| `dockerfile_path` | string | Dockerfile build (real context) |
| `override_image` | string | run this image, skip build |
| `output_directory` | string | static output dir override |
| `build_command` / `start_command` / `pre_deploy_command` | string | lifecycle overrides |
| `entrypoint`, `image` | string | container entry/image overrides |
| `cpus`, `memory` | string | container resource limits |
| `deploy_branches` | string[] | branch rules (`"*"` for all) |
| `preview_environments` | bool | `false` disables preview-env fallback |
| `preview_template` | string | env slug cloned for unmatched branches |
| `deployment_rules.preview_comment` | bool | `false` disables PR comments |
| `health_check` | string \| object | `"/healthz"` or `{path, interval_seconds, failures}` |
| `deployment_retention` | int | keep newest N completed deploys per env |
| `protection_password` | string | write-only; bcrypt'd to Traefik basicAuth on non-prod routers |
| `firewall` | object | `{ip_allowlist: [...], rate_limit: {average, burst}}` |
| `web_analytics.enabled` | bool | first-party pageviews/custom events |
| `speed_insights.enabled` | bool | Core Web Vitals beacon |
| `analytics` | object | third-party provider (`{provider, siteId, …}`) + `gsc_verification` |
| `status_page` | bool \| string | public status page (`true` or custom slug) |
| `webhook_url` | string | deploy event webhook target |

## Environments

`environments` PATCH takes the full array:

```json
[{ "name": "Production", "slug": "prod", "branch": "main", "color": "#22c55e" }]
```

Unmatched branches synthesize a `pv{sha6}` preview environment cloned
from `preview_template` (default `prod`) — env vars scope via the
template's slug.
