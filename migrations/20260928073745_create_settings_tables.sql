CREATE TABLE workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    email TEXT,
    workspace_type TEXT NOT NULL DEFAULT 'BUSINESS'
        CHECK (workspace_type IN ('BUSINESS', 'INDIVIDUAL')),
    created_by_wallet TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE workspace_members (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL
        REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id UUID NOT NULL
        REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'MEMBER'
        CHECK (role IN ('OWNER', 'ADMIN', 'MEMBER')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (workspace_id, user_id)
);

CREATE INDEX workspace_members_user_id_idx
    ON workspace_members(user_id);

CREATE TABLE settings_accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL
        REFERENCES workspaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    pda TEXT NOT NULL UNIQUE,

    -- Stored as NUMERIC because the program uses u128/u64.
    seed NUMERIC(39, 0) NOT NULL,
    settings_authority TEXT NOT NULL,
    threshold INTEGER NOT NULL,
    time_lock BIGINT NOT NULL,
    transaction_index NUMERIC(20, 0) NOT NULL DEFAULT 0,
    stale_transaction_index NUMERIC(20, 0) NOT NULL DEFAULT 0,

    creation_tx_sig TEXT,
    created_by_wallet TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE settings_signers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    settings_account_id UUID NOT NULL
        REFERENCES settings_accounts(id) ON DELETE CASCADE,
    wallet_address TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'EXTERNAL'
        CHECK (role IN ('OWNER', 'AGENT', 'EXTERNAL')),
    permissions_mask INTEGER NOT NULL
        CHECK (permissions_mask BETWEEN 1 AND 7),
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (settings_account_id, wallet_address)
);

CREATE INDEX settings_signers_wallet_address_idx
    ON settings_signers(wallet_address);

CREATE TABLE activity_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE,
    settings_account_id UUID
        REFERENCES settings_accounts(id) ON DELETE CASCADE,
    activity_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    tx_sig TEXT,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX activity_logs_type_tx_sig_unique
    ON activity_logs(activity_type, tx_sig)
    WHERE tx_sig IS NOT NULL;
