use solana_sdk::pubkey::Pubkey;
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::models::{
    ApprovalResponse, PayoutContextResponse, SettingsForTransaction, TransactionRow,
};

pub(super) async fn load_settings(
    state: &AppState,
    settings: &Pubkey,
) -> Result<SettingsForTransaction, ApiError> {
    sqlx::query_as::<_, SettingsForTransaction>(
        r#"SELECT id, workspace_id, pda,
                  stale_transaction_index::text AS stale_transaction_index
           FROM settings_accounts WHERE pda = $1"#,
    )
    .bind(settings.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account is not indexed yet"))
}

pub(super) async fn require_permission(
    state: &AppState,
    settings_id: Uuid,
    wallet: &Pubkey,
    permission: i32,
) -> Result<(), ApiError> {
    let mask = sqlx::query_scalar::<_, i32>(
        "SELECT permissions_mask FROM settings_signers WHERE settings_account_id = $1 AND wallet_address = $2",
    )
    .bind(settings_id)
    .bind(wallet.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Wallet is not a signer for this settings account"))?;
    if mask & permission == 0 {
        return Err(bad_request("Wallet does not have the required permission"));
    }
    Ok(())
}

pub(super) async fn load_approvals(
    state: &AppState,
    transaction_id: Uuid,
) -> Result<Vec<ApprovalResponse>, ApiError> {
    sqlx::query_as::<_, ApprovalResponse>(
        r#"SELECT id, wallet_address, tx_sig, status, created_at, updated_at
           FROM proposal_approvals WHERE transaction_record_id = $1 ORDER BY created_at"#,
    )
    .bind(transaction_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)
}

pub(super) async fn load_payout_context(
    state: &AppState,
    transaction_id: Uuid,
) -> Result<Option<PayoutContextResponse>, ApiError> {
    sqlx::query_as::<_, PayoutContextResponse>(
        r#"SELECT p.id AS payout_id, p.title, p.description, p.category,
                  pr.name AS recipient_name, pr.memo AS recipient_memo
           FROM payout_recipients pr
           JOIN payout_batches p ON p.id = pr.payout_batch_id
           WHERE pr.transaction_record_id = $1"#,
    )
    .bind(transaction_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)
}

pub(super) async fn load_transaction(
    state: &AppState,
    id: &str,
) -> Result<TransactionRow, ApiError> {
    let uuid = Uuid::parse_str(id).ok();
    sqlx::query_as::<_, TransactionRow>(
        r#"SELECT id, settings_account_id, smart_account_id, transaction_pda,
                  proposal_pda, smart_account_pda, transaction_index::text AS transaction_index,
                  account_index, recipient, amount_lamports::text AS amount_lamports, memo,
                  status, create_tx_sig, proposal_tx_sig, execute_tx_sig, created_by_wallet,
                  created_at, updated_at
           FROM transaction_records
           WHERE ($1::uuid IS NOT NULL AND id = $1::uuid)
              OR transaction_pda = $2 OR proposal_pda = $2"#,
    )
    .bind(uuid)
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Transaction not found"))
}
