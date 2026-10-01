ALTER TABLE invoices
    ADD COLUMN public_token_hash TEXT UNIQUE,
    ADD COLUMN sent_at TIMESTAMPTZ,
    ADD COLUMN viewed_at TIMESTAMPTZ;

CREATE TABLE invoice_payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    invoice_id UUID NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    payer_wallet TEXT NOT NULL,
    amount_lamports NUMERIC(20, 0) NOT NULL CHECK (amount_lamports > 0),
    tx_sig TEXT NOT NULL UNIQUE,
    confirmed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX invoice_payments_invoice_id_idx ON invoice_payments(invoice_id, confirmed_at DESC);
