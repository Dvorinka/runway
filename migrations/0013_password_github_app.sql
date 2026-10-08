-- Email+password auth (replaces magic-link/GitHub-OAuth sign-in) and the
-- DB-registered GitHub App (manifest flow) for self-hosted instances.

ALTER TABLE "user" ADD COLUMN IF NOT EXISTS password_hash TEXT;

-- Single-row instance config: credentials returned by GitHub's
-- app-manifest conversion, secrets AES-GCM encrypted like other providers.
CREATE TABLE IF NOT EXISTS github_app (
    id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    app_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    name TEXT,
    client_id TEXT,
    client_secret_enc TEXT,
    pem_enc TEXT NOT NULL,
    webhook_secret_enc TEXT NOT NULL,
    html_url TEXT,
    created_by_user_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
