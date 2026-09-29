CREATE TABLE transaction_records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    settings_account_id UUID NOT NULL REFERENCES settings_accounts(id) ON DELETE CASCADE,
    smart_account_id UUID NOT NULL REFERENCES smart_accounts(id) ON DELETE CASCADE,
    type TEXT NOT NULL DEFAULT 'SOL_TRANSFER',
    transaction_pda TEXT NOT NULL UNIQUE,
    proposal_pda TEXT NOT NULL UNIQUE,
    smart_account_pda TEXT NOT NULL,
    transaction_index NUMERIC(20, 0) NOT NULL,
    account_index INTEGER NOT NULL CHECK (account_index BETWEEN 0 AND 255),
    recipient TEXT NOT NULL,
    amount_lamports NUMERIC(20, 0) NOT NULL CHECK (amount_lamports > 0),
    memo TEXT NOT NULL,
    status TEXT NOT NULL,
    create_tx_sig TEXT NOT NULL,
    proposal_tx_sig TEXT NOT NULL,
    execute_tx_sig TEXT,
    created_by_wallet TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (settings_account_id, transaction_index)
);

CREATE INDEX transaction_records_settings_account_id_idx ON transaction_records(settings_account_id);
CREATE INDEX transaction_records_smart_account_id_idx ON transaction_records(smart_account_id);

CREATE TABLE proposal_approvals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    transaction_record_id UUID NOT NULL REFERENCES transaction_records(id) ON DELETE CASCADE,
    wallet_address TEXT NOT NULL,
    tx_sig TEXT,
    status TEXT NOT NULL DEFAULT 'APPROVED',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (transaction_record_id, wallet_address)
);

CREATE INDEX proposal_approvals_transaction_record_id_idx ON proposal_approvals(transaction_record_id);

ALTER TABLE activity_logs
    ADD COLUMN transaction_record_id UUID REFERENCES transaction_records(id) ON DELETE CASCADE;

CREATE INDEX activity_logs_transaction_record_id_idx ON activity_logs(transaction_record_id);
