CREATE TABLE payout_batches (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    smart_account_id UUID NOT NULL REFERENCES smart_accounts(id),
    title TEXT NOT NULL,
    description TEXT,
    category TEXT NOT NULL DEFAULT 'OTHER'
        CHECK (category IN ('PAYROLL','GRANT','BOUNTY','CONTRACTOR','VENDOR','REIMBURSEMENT','OTHER')),
    status TEXT NOT NULL DEFAULT 'DRAFT'
        CHECK (status IN ('DRAFT','IN_PROGRESS','COMPLETED','PARTIALLY_FAILED','CANCELLED')),
    created_by_wallet TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX payout_batches_workspace_idx ON payout_batches(workspace_id, created_at DESC);
CREATE INDEX payout_batches_smart_account_idx ON payout_batches(smart_account_id);

CREATE TABLE payout_recipients (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    payout_batch_id UUID NOT NULL REFERENCES payout_batches(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL,
    wallet_address TEXT NOT NULL,
    amount_lamports NUMERIC(39, 0) NOT NULL CHECK (amount_lamports > 0),
    memo TEXT,
    transaction_record_id UUID UNIQUE REFERENCES transaction_records(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (payout_batch_id, position)
);

CREATE INDEX payout_recipients_batch_idx ON payout_recipients(payout_batch_id, position);
