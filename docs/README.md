# Runway Docs

Self-hosted deployment platform — one binary for API, workers, and CLI;
Postgres is the only queue; Traefik does edge routing.

## Guides

- [Self-hosting](self-hosting.md) — install, env vars, Cloudflare tunnels, upgrades
- [Framework presets](presets.md) — detection matrix, runners, overrides
- [`runway.json` reference](runway-json.md) — per-repo config and project settings
- [Agents: CLI, API keys, MCP](agents.md) — automation surface
- [User docs](user/README.md) — shipped behavior, user POV
- [Landing changes](agents/landing.md) — verify + PR workflow

## Quickstart

```bash
curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | bash
```

then `runway login`, `runway link`, `runway deploy --follow` from any repo.
