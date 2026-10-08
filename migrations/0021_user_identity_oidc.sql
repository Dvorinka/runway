-- Widen user_identity.provider check: 'oidc' rows are written by the
-- OIDC callback since 0001 but the CHECK only allowed github/google —
-- OIDC sign-ups would violate it on a fresh database.
ALTER TABLE user_identity DROP CONSTRAINT IF EXISTS user_identity_provider_check;
ALTER TABLE user_identity ADD CONSTRAINT user_identity_provider_check
    CHECK (provider IN ('github', 'google', 'oidc'));
