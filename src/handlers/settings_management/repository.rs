use chrono::{Duration, Utc};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error, squadsaccounts::SettingsAccount};

use super::models::{
    IndexedSettings, InviteResponse, InviteRow, SettingsResponse, SettingsRow,
    SettingsSignerResponse, WorkspaceResponse,
};

const INVITE_TTL_DAYS: i64 = 14;

pub(super) struct SignerInviteInput<'a> {
    pub indexed: &'a IndexedSettings,
    pub authority: &'a Pubkey,
    pub signer: &'a Pubkey,
    pub permissions_mask: u8,
    pub name: Option<String>,
    pub email: Option<String>,
    pub designation: Option<String>,
}

pub(super) async fn load_indexed_settings(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<IndexedSettings, ApiError> {
    sqlx::query_as::<_, IndexedSettings>(
        "SELECT id, workspace_id, pda FROM settings_accounts WHERE pda = $1",
    )
    .bind(settings_pda.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account is not indexed yet"))
}

pub(super) async fn find_indexed_settings(
    state: &AppState,
    id_or_pda: &str,
) -> Result<IndexedSettings, ApiError> {
    sqlx::query_as::<_, IndexedSettings>(
        "SELECT id, workspace_id, pda FROM settings_accounts WHERE id::text = $1 OR pda = $1",
    )
    .bind(id_or_pda.trim())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account not found"))
}

pub(super) async fn sync_settings_from_chain(
    state: &AppState,
    indexed: &IndexedSettings,
    chain: &SettingsAccount,
) -> Result<(), ApiError> {
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    sqlx::query(
        r#"UPDATE settings_accounts SET seed = $2::numeric, settings_authority = $3,
           threshold = $4, time_lock = $5, transaction_index = $6::numeric,
           stale_transaction_index = $7::numeric, updated_at = NOW() WHERE id = $1"#,
    )
    .bind(indexed.id)
    .bind(chain.seed.to_string())
    .bind(chain.settings_authority.to_string())
    .bind(i32::from(chain.threshold))
    .bind(i64::from(chain.time_lock))
    .bind(chain.transaction_index.to_string())
    .bind(chain.stale_transaction_index.to_string())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    let mut signer_addresses = Vec::with_capacity(chain.signers.len());
    for (index, signer) in chain.signers.iter().enumerate() {
        let address = signer.key.to_string();
        let role = if signer.key == chain.settings_authority {
            "OWNER"
        } else {
            "EXTERNAL"
        };
        let label = if role == "OWNER" {
            "Authority".to_owned()
        } else {
            format!("Signer {}", index + 1)
        };
        signer_addresses.push(address.clone());
        sqlx::query(
            r#"INSERT INTO settings_signers
               (settings_account_id, wallet_address, role, permissions_mask, label)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (settings_account_id, wallet_address) DO UPDATE SET
               role = EXCLUDED.role, permissions_mask = EXCLUDED.permissions_mask,
               label = COALESCE(settings_signers.label, EXCLUDED.label), updated_at = NOW()"#,
        )
        .bind(indexed.id)
        .bind(address)
        .bind(role)
        .bind(i32::from(signer.permissions.mask))
        .bind(label)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }

    sqlx::query(
        "DELETE FROM settings_signers WHERE settings_account_id = $1 AND NOT (wallet_address = ANY($2))",
    )
    .bind(indexed.id)
    .bind(&signer_addresses)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)
}

pub(super) async fn fetch_settings_response(
    state: &AppState,
    settings_id: Uuid,
) -> Result<SettingsResponse, ApiError> {
    let settings = sqlx::query_as::<_, SettingsRow>(
        r#"SELECT id, workspace_id, name, pda,
        seed::text AS seed, settings_authority, threshold, time_lock,
        transaction_index::text AS transaction_index,
        stale_transaction_index::text AS stale_transaction_index, creation_tx_sig,
        created_by_wallet, created_at, updated_at FROM settings_accounts WHERE id = $1"#,
    )
    .bind(settings_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    let workspace = sqlx::query_as::<_, WorkspaceResponse>(
        "SELECT id, name, workspace_type FROM workspaces WHERE id = $1",
    )
    .bind(settings.workspace_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    let signers = sqlx::query_as::<_, SettingsSignerResponse>(
        r#"SELECT id, wallet_address,
        role, permissions_mask, label FROM settings_signers
        WHERE settings_account_id = $1 ORDER BY created_at ASC"#,
    )
    .bind(settings.id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(SettingsResponse {
        id: settings.id,
        workspace_id: settings.workspace_id,
        name: settings.name,
        pda: settings.pda,
        seed: settings.seed,
        settings_authority: settings.settings_authority,
        threshold: settings.threshold,
        time_lock: settings.time_lock,
        transaction_index: settings.transaction_index,
        stale_transaction_index: settings.stale_transaction_index,
        creation_tx_sig: settings.creation_tx_sig,
        created_by_wallet: settings.created_by_wallet,
        created_at: settings.created_at,
        updated_at: settings.updated_at,
        workspace,
        signers,
    })
}

pub(super) async fn log_activity(
    state: &AppState,
    indexed: &IndexedSettings,
    activity_type: &str,
    title: &str,
    tx_sig: &Signature,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"INSERT INTO activity_logs (workspace_id, settings_account_id,
        activity_type, title, tx_sig, metadata) VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (activity_type, tx_sig) WHERE tx_sig IS NOT NULL DO NOTHING"#,
    )
    .bind(indexed.workspace_id)
    .bind(indexed.id)
    .bind(activity_type)
    .bind(title)
    .bind(tx_sig.to_string())
    .bind(metadata)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(())
}

pub(super) async fn revoke_pending_invites(
    state: &AppState,
    settings_id: Uuid,
    signer: &Pubkey,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"UPDATE workspace_invites SET status = 'REVOKED', updated_at = NOW()
        WHERE settings_account_id = $1 AND wallet_address = $2 AND status = 'PENDING'"#,
    )
    .bind(settings_id)
    .bind(signer.to_string())
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(())
}

pub(super) async fn upsert_signer_profile_and_invite(
    state: &AppState,
    input: SignerInviteInput<'_>,
) -> Result<InviteResponse, ApiError> {
    let signer_address = input.signer.to_string();
    let label = input
        .name
        .clone()
        .or_else(|| input.designation.clone())
        .unwrap_or_else(|| "Signer".to_owned());
    let mut tx = state.db.begin().await.map_err(internal_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{}:{signer_address}", input.indexed.id))
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    let existing =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT user_id FROM wallets WHERE address = $1")
            .bind(&signer_address)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal_error)?
            .flatten();
    let user_id = if let Some(id) = existing {
        sqlx::query_scalar::<_, Uuid>(
            r#"UPDATE users SET name = COALESCE($2, name),
            email = COALESCE($3, email), designation = COALESCE($4, designation),
            updated_at = NOW() WHERE id = $1 RETURNING id"#,
        )
        .bind(id)
        .bind(input.name.as_deref())
        .bind(input.email.as_deref())
        .bind(input.designation.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(internal_error)?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO users (name, email, designation) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(input.name.as_deref())
        .bind(input.email.as_deref())
        .bind(input.designation.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(internal_error)?
    };
    sqlx::query(
        r#"INSERT INTO wallets (address, user_id, label) VALUES ($1, $2, $3)
        ON CONFLICT (address) DO UPDATE SET user_id = EXCLUDED.user_id,
        label = EXCLUDED.label, updated_at = NOW()"#,
    )
    .bind(&signer_address)
    .bind(user_id)
    .bind(&label)
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        r#"INSERT INTO workspace_members (workspace_id, user_id, role)
        VALUES ($1, $2, 'MEMBER') ON CONFLICT (workspace_id, user_id)
        DO UPDATE SET role = 'MEMBER', updated_at = NOW()"#,
    )
    .bind(input.indexed.workspace_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        r#"UPDATE settings_signers SET label = $3, role = 'EXTERNAL',
        permissions_mask = $4, updated_at = NOW()
        WHERE settings_account_id = $1 AND wallet_address = $2"#,
    )
    .bind(input.indexed.id)
    .bind(&signer_address)
    .bind(&label)
    .bind(i32::from(input.permissions_mask))
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;

    let existing_invite = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM workspace_invites
        WHERE settings_account_id = $1 AND wallet_address = $2 AND status = 'PENDING' FOR UPDATE"#,
    )
    .bind(input.indexed.id)
    .bind(&signer_address)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal_error)?;
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);
    let invite = if let Some(id) = existing_invite {
        sqlx::query_as::<_, InviteRow>(
            r#"UPDATE workspace_invites SET email = $2, name = $3,
            designation = $4, permissions_mask = $5, invited_by_wallet = $6,
            expires_at = $7, updated_at = NOW() WHERE id = $1
            RETURNING token, status, wallet_address, email, name, designation,
            permissions_mask, expires_at"#,
        )
        .bind(id)
        .bind(input.email.as_deref())
        .bind(input.name.as_deref())
        .bind(input.designation.as_deref())
        .bind(i32::from(input.permissions_mask))
        .bind(input.authority.to_string())
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal_error)?
    } else {
        let mut bytes = [0_u8; 24];
        getrandom::fill(&mut bytes).map_err(internal_error)?;
        sqlx::query_as::<_, InviteRow>(
            r#"INSERT INTO workspace_invites
            (workspace_id, settings_account_id, token, wallet_address, email, name,
             designation, permissions_mask, invited_by_wallet, expires_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            RETURNING token, status, wallet_address, email, name, designation,
            permissions_mask, expires_at"#,
        )
        .bind(input.indexed.workspace_id)
        .bind(input.indexed.id)
        .bind(hex::encode(bytes))
        .bind(&signer_address)
        .bind(input.email.as_deref())
        .bind(input.name.as_deref())
        .bind(input.designation.as_deref())
        .bind(i32::from(input.permissions_mask))
        .bind(input.authority.to_string())
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal_error)?
    };
    tx.commit().await.map_err(internal_error)?;
    Ok(InviteResponse {
        invite_url: state
            .config
            .frontend_url(&format!("/invite/{}", invite.token)),
        token: invite.token,
        status: invite.status,
        wallet_address: invite.wallet_address,
        email: invite.email,
        name: invite.name,
        designation: invite.designation,
        permissions_mask: invite.permissions_mask,
        expires_at: invite.expires_at,
    })
}
