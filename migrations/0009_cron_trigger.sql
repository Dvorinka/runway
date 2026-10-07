-- Allow 'cron' as a deployment trigger.
-- devpush's cron task passes trigger="cron" but its SQLAlchemy enum only
-- allows ('webhook','user','api') — the value it writes violates its own
-- schema. We extend our CHECK rather than port the bug.
ALTER TABLE deployment DROP CONSTRAINT IF EXISTS deployment_trigger_check;
ALTER TABLE deployment ADD CONSTRAINT deployment_trigger_check
    CHECK (trigger IN ('webhook', 'user', 'api', 'cron'));
