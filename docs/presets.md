# Framework presets

Detection runs on the repo file list; highest-priority match wins.
`output_directory` presets deploy static output (served by an embedded
static server at port 80); the rest run the start command on `PORT`.

| Preset | Name | Runner | Mode |
|---|---|---|---|
| `nextjs` | Next.js | node-20 | SSR (`next start`) |
| `astro-ssr` | Astro (SSR) | node-20 | SSR |
| `astro` | Astro (static) | node-20 | static `dist/` |
| `sveltekit` | SvelteKit | node-20 | SSR |
| `nuxt` | Nuxt | node-20 | SSR (`node .output/server`) |
| `remix` | Remix | node-20 | SSR |
| `vite` | Vite / SPA | node-20 | static `dist/` + SPA fallback |
| `angular` | Angular | node-20 | static + SPA fallback |
| `solidstart` | SolidStart | node-20 | SSR |
| `qwik` | Qwik | node-20 | static + SPA fallback |
| `eleventy` | Eleventy | node-20 | static `_site/` |
| `gatsby` | Gatsby | node-20 | static `public/` |
| `docusaurus` | Docusaurus | node-20 | static `build/` |
| `hugo` | Hugo | go-1.25 | static `public/` |
| `static` | Static (bare HTML) | node-20 | static `.` |
| `flask` | Flask | python-3.12 | server :8000 |
| `django` | Django | python-3.12 | server :8000 |
| `fastapi` | FastAPI | python-3.12 | server :8000 |
| `python` | Python | python-3.12 | server :8000 |
| `nodejs` | Node.js | node-20 | server :8000 |
| `nestjs` | NestJS | node-20 | server :8000 |
| `bun` | Bun | bun-1.3 | server :8000 |
| `go` | Go | go-1.25 | server :8000 |
| `php` | PHP | frankenphp-8.3 | server :8000 |
| `laravel` | Laravel | frankenphp-8.3 | server :8000 |

## Overrides

- `config.preset` — force a preset slug, skip detection.
- `config.dockerfile_path` — build from a Dockerfile instead (real build
  context; `COPY` works). Auto-detected on project create.
- `config.override_image` — skip building entirely, run a published image.
- `config.runner` — swap the runner image slug.
- `data_dir/runner-overrides.json` — instance-level image remap or
  `{"enabled": false}` to block a runner; reread every deploy.

## Package managers

Node presets detect the lockfile (`pnpm-lock.yaml`, `yarn.lock`,
`bun.lockb`) and rewrite npm-flavored commands accordingly.

## Edge behavior

Static output gets immutable caching on hashed-asset paths and
`must-revalidate` on HTML. SSR containers get Traefik routers per
environment/branch alias with the configured firewall/protection
middlewares.
