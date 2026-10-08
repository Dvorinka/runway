-- Speed insights (real-user monitoring) events, per project.
-- Raw events — self-hosted scale doesn't need pre-aggregation.
CREATE TABLE IF NOT EXISTS rum_event (
    id BIGSERIAL PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    host TEXT NOT NULL,
    path TEXT NOT NULL DEFAULT '/',
    lcp DOUBLE PRECISION,
    fcp DOUBLE PRECISION,
    inp DOUBLE PRECISION,
    cls DOUBLE PRECISION,
    ttfb DOUBLE PRECISION,
    ua TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS rum_event_project_ts ON rum_event (project_id, ts DESC);
