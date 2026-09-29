CREATE TABLE receipts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    settings_account_id UUID NOT NULL
        REFERENCES settings_accounts(id) ON DELETE CASCADE,
    smart_account_id UUID
        REFERENCES smart_accounts(id) ON DELETE SET NULL,
    transaction_record_id UUID UNIQUE
        REFERENCES transaction_records(id) ON DELETE SET NULL,
    uploaded_by_wallet TEXT NOT NULL,
    file_url TEXT NOT NULL,
    file_pathname TEXT NOT NULL,
    file_name TEXT,
    file_mime_type TEXT,
    file_size_bytes INTEGER CHECK (file_size_bytes >= 0),
    vendor TEXT,
    amount NUMERIC(39, 9),
    currency TEXT,
    due_date TIMESTAMPTZ,
    invoice_number TEXT,
    category TEXT,
    payment_recipient TEXT,
    confidence DOUBLE PRECISION CHECK (confidence BETWEEN 0 AND 1),
    status TEXT NOT NULL DEFAULT 'UPLOADED'
        CHECK (status IN ('UPLOADED', 'EXTRACTED', 'REVIEWED', 'PAYMENT_PREPARED', 'PAID')),
    raw_ocr_text TEXT,
    extracted_json JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX receipts_settings_account_id_idx ON receipts(settings_account_id);
CREATE INDEX receipts_smart_account_id_idx ON receipts(smart_account_id);
CREATE INDEX receipts_uploaded_by_wallet_idx ON receipts(uploaded_by_wallet);
