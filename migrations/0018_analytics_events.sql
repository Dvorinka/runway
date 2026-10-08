-- First-party web analytics (privacy-preserving).
-- `visitor` is a daily-rotating HMAC of ip+ua — the raw IP is never
-- stored, and the salt window prevents cross-day tracking.
CREATE TABLE IF NOT EXISTS analytics_event (
    id BIGSERIAL PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    host TEXT NOT NULL,
    path TEXT NOT NULL DEFAULT '/',
    kind TEXT NOT NULL DEFAULT 'pageview'
         CHECK (kind IN ('pageview', 'custom')),
    name TEXT,                       -- custom event name
    referrer TEXT NOT NULL DEFAULT '',   -- referrer hostname only
    visitor TEXT NOT NULL DEFAULT '',
    ua TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS analytics_event_project_ts
    ON analytics_event (project_id, ts DESC);
