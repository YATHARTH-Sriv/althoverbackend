CREATE TABLE workspace_invites (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    settings_account_id UUID NOT NULL REFERENCES settings_accounts(id) ON DELETE CASCADE,
    token TEXT NOT NULL UNIQUE,
    wallet_address TEXT NOT NULL,
    email TEXT,
    name TEXT,
    designation TEXT,
    permissions_mask INTEGER NOT NULL CHECK (permissions_mask BETWEEN 1 AND 7),
    invited_by_wallet TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (status IN ('PENDING', 'ACCEPTED', 'REVOKED', 'EXPIRED')),
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX workspace_invites_wallet_address_idx
    ON workspace_invites(wallet_address);

CREATE INDEX workspace_invites_settings_account_id_idx
    ON workspace_invites(settings_account_id);

CREATE UNIQUE INDEX workspace_invites_pending_signer_unique
    ON workspace_invites(settings_account_id, wallet_address)
    WHERE status = 'PENDING';
