-- Phase 5: team invites — port of devpush team_invite (app/models.py ~249).

CREATE TABLE team_invite (
    id          VARCHAR(32) PRIMARY KEY,
    team_id     VARCHAR(32) NOT NULL REFERENCES team(id),
    email       VARCHAR(320) NOT NULL,
    role        TEXT NOT NULL DEFAULT 'member'
                CHECK (role IN ('owner', 'admin', 'member')),
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'accepted', 'revoked')),
    inviter_id  BIGINT NOT NULL REFERENCES "user"(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL DEFAULT now() + interval '30 days'
);
CREATE INDEX ix_team_invite_team ON team_invite (team_id);
CREATE INDEX ix_team_invite_email ON team_invite (email);
