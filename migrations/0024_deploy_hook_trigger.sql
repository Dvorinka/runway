-- Allow 'deploy_hook' as a deployment trigger (tokenized deploy hooks).
ALTER TABLE deployment DROP CONSTRAINT IF EXISTS deployment_trigger_check;
ALTER TABLE deployment ADD CONSTRAINT deployment_trigger_check
    CHECK (trigger IN ('webhook', 'user', 'api', 'cron', 'deploy_hook'));
