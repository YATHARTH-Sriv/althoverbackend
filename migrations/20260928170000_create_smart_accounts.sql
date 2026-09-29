CREATE TABLE smart_accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    settings_account_id UUID NOT NULL
        REFERENCES settings_accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    account_index INTEGER NOT NULL
        CHECK (account_index BETWEEN 0 AND 255),
    pda TEXT NOT NULL UNIQUE,
    balance_lamports NUMERIC(20, 0) NOT NULL DEFAULT 0,
    created_by_wallet TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (settings_account_id, account_index)
);

CREATE INDEX smart_accounts_settings_account_id_idx
    ON smart_accounts(settings_account_id);

ALTER TABLE activity_logs
    ADD COLUMN smart_account_id UUID
        REFERENCES smart_accounts(id) ON DELETE CASCADE;

CREATE INDEX activity_logs_smart_account_id_idx
    ON activity_logs(smart_account_id);
