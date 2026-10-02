CREATE TABLE invoice_sequences (
    workspace_id UUID PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    next_value BIGINT NOT NULL DEFAULT 1 CHECK (next_value > 0)
);

CREATE TABLE invoices (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL REFERENCES customers(id),
    receiving_smart_account_id UUID NOT NULL REFERENCES smart_accounts(id),
    invoice_number BIGINT NOT NULL CHECK (invoice_number > 0),
    status TEXT NOT NULL DEFAULT 'DRAFT'
        CHECK (status IN ('DRAFT', 'SENT', 'VIEWED', 'PARTIALLY_PAID', 'PAID', 'OVERDUE', 'VOID')),
    currency TEXT NOT NULL DEFAULT 'SOL' CHECK (currency = 'SOL'),
    issue_date DATE NOT NULL,
    due_date DATE,
    customer_business_name TEXT NOT NULL,
    customer_billing_email TEXT,
    customer_contact_name TEXT,
    customer_wallet_address TEXT,
    memo TEXT,
    subtotal NUMERIC(39, 9) NOT NULL CHECK (subtotal >= 0),
    total NUMERIC(39, 9) NOT NULL CHECK (total >= 0),
    amount_paid NUMERIC(39, 9) NOT NULL DEFAULT 0 CHECK (amount_paid >= 0),
    created_by_wallet TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workspace_id, invoice_number),
    CHECK (due_date IS NULL OR due_date >= issue_date),
    CHECK (amount_paid <= total)
);

CREATE INDEX invoices_workspace_id_idx ON invoices(workspace_id);
CREATE INDEX invoices_customer_id_idx ON invoices(customer_id);
CREATE INDEX invoices_receiving_smart_account_id_idx ON invoices(receiving_smart_account_id);
CREATE INDEX invoices_status_idx ON invoices(workspace_id, status);

CREATE TABLE invoice_line_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    invoice_id UUID NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    description TEXT NOT NULL,
    quantity NUMERIC(20, 6) NOT NULL CHECK (quantity > 0),
    unit_price NUMERIC(39, 9) NOT NULL CHECK (unit_price >= 0),
    amount NUMERIC(39, 9) NOT NULL CHECK (amount >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (invoice_id, position)
);

CREATE INDEX invoice_line_items_invoice_id_idx ON invoice_line_items(invoice_id);
