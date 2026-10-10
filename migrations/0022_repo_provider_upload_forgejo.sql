-- Widen repo_provider checks: 'forgejo' (Gitea-API family) and 'upload'
-- (local-directory projects deployed via tarball) are written by project
-- creation but the CHECKs only allowed the original five providers.
-- Written re-runnable (DROP + conditional ADD) so manual applies and
-- migration runs converge.
DO $$ BEGIN
    ALTER TABLE project DROP CONSTRAINT IF EXISTS project_repo_provider_check;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'project_repo_provider_check') THEN
        ALTER TABLE project ADD CONSTRAINT project_repo_provider_check
            CHECK (repo_provider IN ('github', 'gitea', 'forgejo', 'gitlab', 'bitbucket', 'github_enterprise', 'upload'));
    END IF;
    ALTER TABLE deployment DROP CONSTRAINT IF EXISTS deployment_repo_provider_check;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'deployment_repo_provider_check') THEN
        ALTER TABLE deployment ADD CONSTRAINT deployment_repo_provider_check
            CHECK (repo_provider IN ('github', 'gitea', 'forgejo', 'gitlab', 'bitbucket', 'github_enterprise', 'upload'));
    END IF;
END $$;
