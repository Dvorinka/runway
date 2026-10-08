-- TOTP two-factor auth: encrypted secret, enable flag, recovery-code
-- hashes (sha256 — codes are shown once, stored like API keys).
ALTER TABLE "user" ADD COLUMN IF NOT EXISTS totp_secret_enc TEXT;
ALTER TABLE "user" ADD COLUMN IF NOT EXISTS totp_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE "user" ADD COLUMN IF NOT EXISTS totp_recovery JSONB NOT NULL DEFAULT '[]'::jsonb;
