use std::collections::HashMap;

use solana_sdk::pubkey::Pubkey;
use uuid::Uuid;

use crate::{
    ApiError, AppState, internal_error,
    squadsaccounts::{ProposalAccount, decode_proposal_account},
    validation::parse_pubkey,
};

use super::super::receipts::mark_transaction_receipt_paid;
use super::models::*;
use super::repository::*;

pub(super) async fn fetch_proposal(
    state: &AppState,
    proposal: &Pubkey,
) -> Result<ProposalAccount, ApiError> {
    let account = state
        .rpc
        .get_account(proposal)
        .await
        .map_err(internal_error)?;
    decode_proposal_account(&account.owner, &account.data).map_err(internal_error)
}

pub(super) fn proposal_response(proposal: &ProposalAccount) -> ProposalResponse {
    ProposalResponse {
        settings: proposal.settings.to_string(),
        transaction_index: proposal.transaction_index.to_string(),
        status: proposal.status.database_value().to_owned(),
        approved: proposal.approved.iter().map(ToString::to_string).collect(),
        rejected: proposal.rejected.iter().map(ToString::to_string).collect(),
        cancelled: proposal.cancelled.iter().map(ToString::to_string).collect(),
    }
}

pub(super) async fn transaction_response(
    state: &AppState,
    row: TransactionRow,
    stale_index: u64,
) -> Result<TransactionResponse, ApiError> {
    let approvals = load_approvals(state, row.id).await?;
    let payout = load_payout_context(state, row.id).await?;
    transaction_response_with_context(row, stale_index, approvals, payout)
}

fn transaction_response_with_context(
    row: TransactionRow,
    stale_index: u64,
    approvals: Vec<ApprovalResponse>,
    payout: Option<PayoutContextResponse>,
) -> Result<TransactionResponse, ApiError> {
    let is_stale = row.status == "PROPOSAL_ACTIVE"
        && row
            .transaction_index
            .parse::<u64>()
            .map_err(internal_error)?
            <= stale_index;
    Ok(TransactionResponse {
        id: row.id,
        settings_account_id: row.settings_account_id,
        smart_account_id: row.smart_account_id,
        transaction_type: "SOL_TRANSFER",
        transaction_pda: row.transaction_pda,
        proposal_pda: row.proposal_pda,
        smart_account_pda: row.smart_account_pda,
        transaction_index: row.transaction_index,
        account_index: row.account_index,
        recipient: row.recipient,
        amount_lamports: row.amount_lamports,
        memo: row.memo,
        status: row.status,
        create_tx_sig: row.create_tx_sig,
        proposal_tx_sig: row.proposal_tx_sig,
        execute_tx_sig: row.execute_tx_sig,
        created_by_wallet: row.created_by_wallet,
        created_at: row.created_at,
        updated_at: row.updated_at,
        approvals,
        is_stale,
        stale_reason: is_stale.then_some("Settings changed after this proposal was created"),
        payout,
    })
}

pub(crate) async fn load_transactions_for_dashboard(
    state: &AppState,
    settings_id: Uuid,
    stale_index: u64,
    refresh: bool,
) -> Result<Vec<TransactionResponse>, ApiError> {
    let mut rows = sqlx::query_as::<_, TransactionRow>(
        r#"SELECT id, settings_account_id, smart_account_id, transaction_pda,
                  proposal_pda, smart_account_pda, transaction_index::text AS transaction_index,
                  account_index, recipient, amount_lamports::text AS amount_lamports, memo,
                  status, create_tx_sig, proposal_tx_sig, execute_tx_sig, created_by_wallet,
                  created_at, updated_at
           FROM transaction_records
           WHERE settings_account_id = $1
           ORDER BY transaction_index DESC
           LIMIT 100"#,
    )
    .bind(settings_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    if refresh {
        let mut refreshed = Vec::with_capacity(rows.len());
        for row in rows {
            refreshed.push(sync_proposal(state, &row).await?.0);
        }
        rows = refreshed;
    }

    let transaction_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let approval_rows = sqlx::query_as::<_, ApprovalWithTransaction>(
        r#"SELECT transaction_record_id, id, wallet_address, tx_sig, status,
                  created_at, updated_at
           FROM proposal_approvals
           WHERE transaction_record_id = ANY($1)
           ORDER BY created_at"#,
    )
    .bind(&transaction_ids)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let payout_rows = sqlx::query_as::<_, PayoutWithTransaction>(
        r#"SELECT pr.transaction_record_id, p.id AS payout_id, p.title,
                  p.description, p.category, pr.name AS recipient_name,
                  pr.memo AS recipient_memo
           FROM payout_recipients pr
           JOIN payout_batches p ON p.id = pr.payout_batch_id
           WHERE pr.transaction_record_id = ANY($1)"#,
    )
    .bind(&transaction_ids)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    let mut approvals_by_transaction: HashMap<Uuid, Vec<ApprovalResponse>> = HashMap::new();
    for approval in approval_rows {
        approvals_by_transaction
            .entry(approval.transaction_record_id)
            .or_default()
            .push(ApprovalResponse {
                id: approval.id,
                wallet_address: approval.wallet_address,
                tx_sig: approval.tx_sig,
                status: approval.status,
                created_at: approval.created_at,
                updated_at: approval.updated_at,
            });
    }
    let payouts_by_transaction: HashMap<Uuid, PayoutContextResponse> = payout_rows
        .into_iter()
        .map(|payout| {
            (
                payout.transaction_record_id,
                PayoutContextResponse {
                    payout_id: payout.payout_id,
                    title: payout.title,
                    description: payout.description,
                    category: payout.category,
                    recipient_name: payout.recipient_name,
                    recipient_memo: payout.recipient_memo,
                },
            )
        })
        .collect();

    rows.into_iter()
        .map(|row| {
            let id = row.id;
            transaction_response_with_context(
                row,
                stale_index,
                approvals_by_transaction.remove(&id).unwrap_or_default(),
                payouts_by_transaction.get(&id).cloned(),
            )
        })
        .collect()
}

pub(super) async fn sync_proposal(
    state: &AppState,
    row: &TransactionRow,
) -> Result<(TransactionRow, ProposalAccount), ApiError> {
    let proposal_pda = parse_pubkey(&row.proposal_pda, "proposalPda")?;
    let proposal = fetch_proposal(state, &proposal_pda).await?;
    for wallet in &proposal.approved {
        sqlx::query(
            r#"INSERT INTO proposal_approvals (transaction_record_id, wallet_address, status)
               VALUES ($1, $2, 'APPROVED') ON CONFLICT (transaction_record_id, wallet_address)
               DO UPDATE SET status = 'APPROVED', updated_at = NOW()"#,
        )
        .bind(row.id)
        .bind(wallet.to_string())
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    }
    for wallet in &proposal.rejected {
        sqlx::query(
            r#"INSERT INTO proposal_approvals (transaction_record_id, wallet_address, status)
               VALUES ($1, $2, 'REJECTED') ON CONFLICT (transaction_record_id, wallet_address)
               DO UPDATE SET status = 'REJECTED', updated_at = NOW()"#,
        )
        .bind(row.id)
        .bind(wallet.to_string())
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    }
    let updated = sqlx::query_as::<_, TransactionRow>(
        r#"UPDATE transaction_records SET status = $2, updated_at = NOW() WHERE id = $1
           RETURNING id, settings_account_id, smart_account_id, transaction_pda,
                     proposal_pda, smart_account_pda, transaction_index::text AS transaction_index,
                     account_index, recipient, amount_lamports::text AS amount_lamports, memo,
                     status, create_tx_sig, proposal_tx_sig, execute_tx_sig, created_by_wallet,
                     created_at, updated_at"#,
    )
    .bind(row.id)
    .bind(proposal.status.database_value())
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    if proposal.status.database_value() == "EXECUTED" {
        mark_transaction_receipt_paid(state, updated.id).await?;
    }
    Ok((updated, proposal))
}
