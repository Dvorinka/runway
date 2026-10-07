-- Phase 5: user-scoped git provider connections — port of devpush
-- gitea_connection (~279), gitlab_connection (~315),
-- bitbucket_connection (~1680). Tokens are AES-GCM ciphertext.

CREATE TABLE gitea_connection (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES "user"(id),
    base_url   VARCHAR(512) NOT NULL,
    username   VARCHAR(255) NOT NULL,
    token      VARCHAR(2048) NOT NULL,   -- AES-GCM ciphertext
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, base_url)
);

CREATE TABLE gitlab_connection (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES "user"(id),
    base_url   VARCHAR(512) NOT NULL,
    username   VARCHAR(255) NOT NULL,
    token      VARCHAR(2048) NOT NULL,   -- AES-GCM ciphertext
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, base_url)
);

CREATE TABLE bitbucket_connection (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    workspace  VARCHAR(100) NOT NULL,
    token      VARCHAR(512) NOT NULL,    -- AES-GCM ciphertext
    token_type VARCHAR(20) NOT NULL DEFAULT 'app',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, workspace)
);

ALTER TABLE project
    ADD CONSTRAINT fk_project_gitea_conn
        FOREIGN KEY (gitea_connection_id) REFERENCES gitea_connection(id),
    ADD CONSTRAINT fk_project_gitlab_conn
        FOREIGN KEY (gitlab_connection_id) REFERENCES gitlab_connection(id);
