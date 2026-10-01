CREATE TABLE customers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL
        REFERENCES workspaces(id) ON DELETE CASCADE,
    business_name TEXT NOT NULL
        CHECK (length(btrim(business_name)) BETWEEN 1 AND 160),
    billing_email TEXT,
    contact_name TEXT,
    wallet_address TEXT,
    notes TEXT,
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX customers_workspace_id_idx
    ON customers(workspace_id);

CREATE INDEX customers_active_workspace_name_idx
    ON customers(workspace_id, lower(business_name))
    WHERE archived_at IS NULL;

CREATE INDEX customers_workspace_billing_email_idx
    ON customers(workspace_id, lower(billing_email))
    WHERE billing_email IS NOT NULL;
