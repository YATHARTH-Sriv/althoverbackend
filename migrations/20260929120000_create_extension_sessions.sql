CREATE TABLE extension_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash TEXT UNIQUE,
    exchange_code_hash TEXT UNIQUE,
    code_challenge TEXT NOT NULL,
    extension_id TEXT,
    wallet_address TEXT NOT NULL
        REFERENCES wallets(address) ON DELETE CASCADE,
    settings_account_id UUID NOT NULL
        REFERENCES settings_accounts(id) ON DELETE CASCADE,
    name TEXT,
    revoked_at TIMESTAMPTZ,
    exchanged_at TIMESTAMPTZ,
    exchange_expires_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CHECK (token_hash IS NOT NULL OR exchange_code_hash IS NOT NULL)
);

CREATE INDEX extension_sessions_wallet_address_idx
    ON extension_sessions(wallet_address);

CREATE INDEX extension_sessions_settings_account_id_idx
    ON extension_sessions(settings_account_id);

CREATE INDEX extension_sessions_expires_at_idx
    ON extension_sessions(expires_at);

CREATE INDEX extension_sessions_exchange_expires_at_idx
    ON extension_sessions(exchange_expires_at);
