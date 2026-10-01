use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        build_unsigned_transaction_base64, derive_proposal_pda, derive_smart_account_pda,
        derive_transaction_pda, verify_confirmed_squads_instruction,
    },
    squadsaccounts::{
        ProposalAccount, build_approve_proposal_instruction, build_create_proposal_instruction,
        build_create_transaction_instruction, build_execute_transaction_instruction,
        build_reject_proposal_instruction, decode_proposal_account, decode_settings_account,
        decode_transaction_message,
    },
    validation::{parse_pubkey, parse_signature},
};

use super::receipts::{link_receipt_to_transaction, mark_transaction_receipt_paid};

const INITIATE_PERMISSION: i32 = 1;
const VOTE_PERMISSION: i32 = 2;
const EXECUTE_PERMISSION: i32 = 4;

#[derive(Debug, FromRow)]
struct SettingsForTransaction {
    id: Uuid,
    workspace_id: Uuid,
    pda: String,
    stale_transaction_index: String,
}

#[derive(Debug, FromRow)]
struct ApprovalWithTransaction {
    transaction_record_id: Uuid,
    id: Uuid,
    wallet_address: String,
    tx_sig: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct PayoutWithTransaction {
    transaction_record_id: Uuid,
    payout_id: Uuid,
    title: String,
    description: Option<String>,
    category: String,
    recipient_name: String,
    recipient_memo: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TransactionRow {
    pub(crate) id: Uuid,
    pub(crate) settings_account_id: Uuid,
    pub(crate) smart_account_id: Uuid,
    pub(crate) transaction_pda: String,
    pub(crate) proposal_pda: String,
    pub(crate) smart_account_pda: String,
    pub(crate) transaction_index: String,
    pub(crate) account_index: i32,
    pub(crate) recipient: String,
    pub(crate) amount_lamports: String,
    pub(crate) memo: String,
    pub(crate) status: String,
    pub(crate) create_tx_sig: String,
    pub(crate) proposal_tx_sig: String,
    pub(crate) execute_tx_sig: Option<String>,
    pub(crate) created_by_wallet: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalResponse {
    id: Uuid,
    wallet_address: String,
    tx_sig: Option<String>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PayoutContextResponse {
    payout_id: Uuid,
    title: String,
    description: Option<String>,
    category: String,
    recipient_name: String,
    recipient_memo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionResponse {
    pub(crate) id: Uuid,
    pub(crate) settings_account_id: Uuid,
    pub(crate) smart_account_id: Uuid,
    #[serde(rename = "type")]
    pub(crate) transaction_type: &'static str,
    pub(crate) transaction_pda: String,
    pub(crate) proposal_pda: String,
    pub(crate) smart_account_pda: String,
    pub(crate) transaction_index: String,
    pub(crate) account_index: i32,
    pub(crate) recipient: String,
    pub(crate) amount_lamports: String,
    pub(crate) memo: String,
    pub(crate) status: String,
    pub(crate) create_tx_sig: String,
    pub(crate) proposal_tx_sig: String,
    pub(crate) execute_tx_sig: Option<String>,
    pub(crate) created_by_wallet: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) approvals: Vec<ApprovalResponse>,
    pub(crate) is_stale: bool,
    pub(crate) stale_reason: Option<&'static str>,
    pub(crate) payout: Option<PayoutContextResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IntegerInput {
    Text(String),
    Number(u64),
}

impl IntegerInput {
    fn parse(self, field: &str, allow_zero: bool) -> Result<u64, ApiError> {
        let value = match self {
            Self::Text(value) => value.trim().parse::<u64>(),
            Self::Number(value) => Ok(value),
        }
        .map_err(|_| bad_request(format!("{field} must be an unsigned integer")))?;
        if !allow_zero && value == 0 {
            return Err(bad_request(format!("{field} must be greater than 0")));
        }
        Ok(value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateTransactionRequest {
    wallet_address: String,
    settings_pda: String,
    recipient: String,
    account_index: i32,
    amount_lamports: IntegerInput,
    memo: Option<String>,
    smart_account_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateTransactionResponse {
    wallet_address: String,
    settings_pda: String,
    account_index: u8,
    recipient: String,
    amount_lamports: String,
    memo: String,
    smart_account_name: Option<String>,
    smart_account_pda: String,
    transaction_pda: String,
    transaction_index: String,
    transaction_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProposalRequest {
    wallet_address: String,
    settings_pda: String,
    transaction_index: IntegerInput,
    draft: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProposalResponse {
    wallet_address: String,
    settings_pda: String,
    transaction_index: String,
    proposal_pda: String,
    transaction_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletSubmittedRequest {
    wallet_address: String,
    settings_pda: String,
    recipient: String,
    account_index: i32,
    transaction_index: IntegerInput,
    transaction_pda: String,
    proposal_pda: String,
    smart_account_pda: String,
    amount_lamports: IntegerInput,
    create_tx_sig: String,
    proposal_tx_sig: String,
    memo: Option<String>,
    smart_account_name: Option<String>,
    receipt_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletQuery {
    wallet_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedActionRequest {
    wallet_address: String,
    tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltActionResponse {
    transaction_id: Uuid,
    wallet_address: String,
    transaction_base64: String,
    proposal_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    smart_account_pda: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransactionEnvelope {
    transaction: TransactionResponse,
}

#[derive(Debug, Serialize)]
pub struct TransactionProposalEnvelope {
    transaction: TransactionResponse,
    proposal: ProposalResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalResponse {
    settings: String,
    transaction_index: String,
    status: String,
    approved: Vec<String>,
    rejected: Vec<String>,
    cancelled: Vec<String>,
}

fn normalize_memo(value: Option<String>) -> String {
    value
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_owned()))
        .unwrap_or_else(|| "Hover treasury payment".to_owned())
}

async fn load_settings(
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

async fn require_permission(
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

async fn fetch_proposal(state: &AppState, proposal: &Pubkey) -> Result<ProposalAccount, ApiError> {
    let account = state
        .rpc
        .get_account(proposal)
        .await
        .map_err(internal_error)?;
    decode_proposal_account(&account.owner, &account.data).map_err(internal_error)
}

fn proposal_response(proposal: &ProposalAccount) -> ProposalResponse {
    ProposalResponse {
        settings: proposal.settings.to_string(),
        transaction_index: proposal.transaction_index.to_string(),
        status: proposal.status.database_value().to_owned(),
        approved: proposal.approved.iter().map(ToString::to_string).collect(),
        rejected: proposal.rejected.iter().map(ToString::to_string).collect(),
        cancelled: proposal.cancelled.iter().map(ToString::to_string).collect(),
    }
}

async fn load_approvals(
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

async fn load_payout_context(
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

pub(crate) async fn transaction_response(
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

async fn load_transaction(state: &AppState, id: &str) -> Result<TransactionRow, ApiError> {
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

async fn sync_proposal(
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

pub async fn build_create_transaction(
    State(state): State<AppState>,
    Json(payload): Json<BuildCreateTransactionRequest>,
) -> Result<Json<BuildCreateTransactionResponse>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let recipient = parse_pubkey(&payload.recipient, "recipient")?;
    let account_index = u8::try_from(payload.account_index)
        .map_err(|_| bad_request("accountIndex must be between 0 and 255"))?;
    let amount = payload.amount_lamports.parse("amountLamports", false)?;
    let memo = normalize_memo(payload.memo);
    let settings = load_settings(&state, &settings_pda).await?;
    require_permission(&state, settings.id, &wallet, INITIATE_PERMISSION).await?;
    let chain_settings_account = state
        .rpc
        .get_account(&settings_pda)
        .await
        .map_err(internal_error)?;
    let chain_settings =
        decode_settings_account(&chain_settings_account.owner, &chain_settings_account.data)
            .map_err(internal_error)?;
    let transaction_index = chain_settings
        .transaction_index
        .checked_add(1)
        .ok_or_else(|| bad_request("Transaction index overflow"))?;
    let (transaction_pda, smart_account_pda, instruction) = build_create_transaction_instruction(
        settings_pda,
        wallet,
        transaction_index,
        account_index,
        recipient,
        amount,
        Some(memo.clone()),
    )
    .map_err(internal_error)?;
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    let transaction_base64 = build_unsigned_transaction_base64(wallet, blockhash, instruction)
        .map_err(internal_error)?;
    Ok(Json(BuildCreateTransactionResponse {
        wallet_address: wallet.to_string(),
        settings_pda: settings.pda,
        account_index,
        recipient: recipient.to_string(),
        amount_lamports: amount.to_string(),
        memo,
        smart_account_name: payload.smart_account_name,
        smart_account_pda: smart_account_pda.to_string(),
        transaction_pda: transaction_pda.to_string(),
        transaction_index: transaction_index.to_string(),
        transaction_base64,
    }))
}

pub async fn build_create_proposal(
    State(state): State<AppState>,
    Json(payload): Json<BuildProposalRequest>,
) -> Result<Json<BuildProposalResponse>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let transaction_index = payload.transaction_index.parse("transactionIndex", true)?;
    let settings = load_settings(&state, &settings_pda).await?;
    require_permission(&state, settings.id, &wallet, INITIATE_PERMISSION).await?;
    let (proposal_pda, instruction) = build_create_proposal_instruction(
        settings_pda,
        wallet,
        transaction_index,
        payload.draft.unwrap_or(false),
    )
    .map_err(internal_error)?;
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    let transaction_base64 = build_unsigned_transaction_base64(wallet, blockhash, instruction)
        .map_err(internal_error)?;
    Ok(Json(BuildProposalResponse {
        wallet_address: wallet.to_string(),
        settings_pda: settings_pda.to_string(),
        transaction_index: transaction_index.to_string(),
        proposal_pda: proposal_pda.to_string(),
        transaction_base64,
    }))
}

pub async fn wallet_transaction_submitted(
    State(state): State<AppState>,
    Json(payload): Json<WalletSubmittedRequest>,
) -> Result<Json<TransactionProposalEnvelope>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let recipient = parse_pubkey(&payload.recipient, "recipient")?;
    let account_index = u8::try_from(payload.account_index)
        .map_err(|_| bad_request("accountIndex must be between 0 and 255"))?;
    let transaction_index = payload.transaction_index.parse("transactionIndex", true)?;
    let amount = payload.amount_lamports.parse("amountLamports", false)?;
    let transaction_pda = parse_pubkey(&payload.transaction_pda, "transactionPda")?;
    let proposal_pda = parse_pubkey(&payload.proposal_pda, "proposalPda")?;
    let smart_account_pda = parse_pubkey(&payload.smart_account_pda, "smartAccountPda")?;
    if derive_transaction_pda(&settings_pda, transaction_index).0 != transaction_pda {
        return Err(bad_request(
            "transactionPda does not match settingsPda and transactionIndex",
        ));
    }
    if derive_proposal_pda(&settings_pda, transaction_index).0 != proposal_pda {
        return Err(bad_request(
            "proposalPda does not match settingsPda and transactionIndex",
        ));
    }
    if derive_smart_account_pda(&settings_pda, account_index).0 != smart_account_pda {
        return Err(bad_request(
            "smartAccountPda does not match settingsPda and accountIndex",
        ));
    }
    let settings = load_settings(&state, &settings_pda).await?;
    require_permission(&state, settings.id, &wallet, INITIATE_PERMISSION).await?;
    let create_sig = parse_signature(&payload.create_tx_sig, "createTxSig")?;
    let proposal_sig = parse_signature(&payload.proposal_tx_sig, "proposalTxSig")?;
    verify_confirmed_squads_instruction(
        &state.rpc,
        &create_sig,
        &wallet,
        &[settings_pda, transaction_pda],
        "global:create_transaction",
    )
    .await
    .map_err(bad_request)?;
    verify_confirmed_squads_instruction(
        &state.rpc,
        &proposal_sig,
        &wallet,
        &[settings_pda, proposal_pda],
        "global:create_proposal",
    )
    .await
    .map_err(bad_request)?;
    let proposal = fetch_proposal(&state, &proposal_pda).await?;
    if proposal.settings != settings_pda || proposal.transaction_index != transaction_index {
        return Err(bad_request(
            "Proposal account does not match this transaction",
        ));
    }
    let smart_account_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM smart_accounts WHERE settings_account_id = $1 AND pda = $2",
    )
    .bind(settings.id)
    .bind(smart_account_pda.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Smart account is not indexed yet"))?;
    let memo = normalize_memo(payload.memo);
    let row = sqlx::query_as::<_, TransactionRow>(
        r#"INSERT INTO transaction_records (
        settings_account_id, smart_account_id, transaction_pda, proposal_pda, smart_account_pda,
        transaction_index, account_index, recipient, amount_lamports, memo, status,
        create_tx_sig, proposal_tx_sig, created_by_wallet)
        VALUES ($1,$2,$3,$4,$5,$6::numeric,$7,$8,$9::numeric,$10,$11,$12,$13,$14)
        ON CONFLICT (transaction_pda) DO UPDATE SET proposal_pda = EXCLUDED.proposal_pda,
        proposal_tx_sig = EXCLUDED.proposal_tx_sig, status = EXCLUDED.status, updated_at = NOW()
        RETURNING id, settings_account_id, smart_account_id, transaction_pda,
                  proposal_pda, smart_account_pda, transaction_index::text AS transaction_index,
                  account_index, recipient, amount_lamports::text AS amount_lamports, memo,
                  status, create_tx_sig, proposal_tx_sig, execute_tx_sig, created_by_wallet,
                  created_at, updated_at"#,
    )
    .bind(settings.id)
    .bind(smart_account_id)
    .bind(transaction_pda.to_string())
    .bind(proposal_pda.to_string())
    .bind(smart_account_pda.to_string())
    .bind(transaction_index.to_string())
    .bind(i32::from(account_index))
    .bind(recipient.to_string())
    .bind(amount.to_string())
    .bind(memo)
    .bind(proposal.status.database_value())
    .bind(create_sig.to_string())
    .bind(proposal_sig.to_string())
    .bind(wallet.to_string())
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    sqlx::query("UPDATE settings_accounts SET transaction_index = GREATEST(transaction_index, $2::numeric), updated_at = NOW() WHERE id = $1")
        .bind(settings.id).bind(transaction_index.to_string()).execute(&state.db).await.map_err(internal_error)?;
    sqlx::query(r#"INSERT INTO activity_logs (workspace_id, settings_account_id, smart_account_id, transaction_record_id, activity_type, title, tx_sig)
        VALUES ($1,$2,$3,$4,'TRANSACTION_CREATED','Transaction proposal created',$5)
        ON CONFLICT DO NOTHING"#)
        .bind(settings.workspace_id).bind(settings.id).bind(smart_account_id).bind(row.id).bind(create_sig.to_string())
        .execute(&state.db).await.map_err(internal_error)?;
    if let Some(receipt_id) = payload.receipt_id {
        link_receipt_to_transaction(&state, receipt_id, settings.id, smart_account_id, row.id)
            .await?;
    }
    let stale_index = settings
        .stale_transaction_index
        .parse::<u64>()
        .map_err(internal_error)?;
    let response = transaction_response(&state, row, stale_index).await?;
    let _ = payload.smart_account_name;
    Ok(Json(TransactionProposalEnvelope {
        transaction: response,
        proposal: proposal_response(&proposal),
    }))
}

pub async fn refresh_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TransactionEnvelope>, ApiError> {
    let row = load_transaction(&state, &id).await?;
    let (row, _) = sync_proposal(&state, &row).await?;
    let stale = sqlx::query_scalar::<_, String>(
        "SELECT stale_transaction_index::text FROM settings_accounts WHERE id = $1",
    )
    .bind(row.settings_account_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?
    .parse::<u64>()
    .map_err(internal_error)?;
    Ok(Json(TransactionEnvelope {
        transaction: transaction_response(&state, row, stale).await?,
    }))
}

pub async fn build_approve_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WalletQuery>,
) -> Result<Json<BuiltActionResponse>, ApiError> {
    let wallet = parse_pubkey(
        query.wallet_address.as_deref().unwrap_or(""),
        "walletAddress",
    )?;
    let row = load_transaction(&state, &id).await?;
    let settings = sqlx::query_as::<_, SettingsForTransaction>(r#"SELECT id, workspace_id, pda, stale_transaction_index::text AS stale_transaction_index FROM settings_accounts WHERE id = $1"#).bind(row.settings_account_id).fetch_one(&state.db).await.map_err(internal_error)?;
    require_permission(&state, settings.id, &wallet, VOTE_PERMISSION).await?;
    let stale = settings
        .stale_transaction_index
        .parse::<u64>()
        .map_err(internal_error)?;
    let index = row
        .transaction_index
        .parse::<u64>()
        .map_err(internal_error)?;
    if row.status == "PROPOSAL_ACTIVE" && index <= stale {
        return Err(bad_request(
            "This proposal became stale after the signer or threshold settings changed. Create a new transaction proposal.",
        ));
    }
    if row.status != "PROPOSAL_ACTIVE" {
        return Err(bad_request(format!(
            "A {} proposal cannot be approved",
            row.status.to_lowercase()
        )));
    }
    let settings_pda = parse_pubkey(&settings.pda, "settingsPda")?;
    let (proposal_pda, instruction) =
        build_approve_proposal_instruction(settings_pda, wallet, index).map_err(internal_error)?;
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    Ok(Json(BuiltActionResponse {
        transaction_id: row.id,
        wallet_address: wallet.to_string(),
        transaction_base64: build_unsigned_transaction_base64(wallet, blockhash, instruction)
            .map_err(internal_error)?,
        proposal_pda: proposal_pda.to_string(),
        smart_account_pda: None,
    }))
}

pub async fn approval_submitted(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SubmittedActionRequest>,
) -> Result<Json<TransactionProposalEnvelope>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let sig = parse_signature(&payload.tx_sig, "txSig")?;
    let row = load_transaction(&state, &id).await?;
    let settings_pda = parse_pubkey(
        &sqlx::query_scalar::<_, String>("SELECT pda FROM settings_accounts WHERE id = $1")
            .bind(row.settings_account_id)
            .fetch_one(&state.db)
            .await
            .map_err(internal_error)?,
        "settingsPda",
    )?;
    let proposal_pda = parse_pubkey(&row.proposal_pda, "proposalPda")?;
    verify_confirmed_squads_instruction(
        &state.rpc,
        &sig,
        &wallet,
        &[settings_pda, proposal_pda],
        "global:approve_proposal",
    )
    .await
    .map_err(bad_request)?;
    let proposal = fetch_proposal(&state, &proposal_pda).await?;
    if !proposal.approved.contains(&wallet) {
        return Err(bad_request(
            "Confirmed transaction did not record the requested approval",
        ));
    }
    sqlx::query(
        r#"INSERT INTO proposal_approvals (transaction_record_id, wallet_address, tx_sig, status)
        VALUES ($1,$2,$3,'APPROVED') ON CONFLICT (transaction_record_id, wallet_address)
        DO UPDATE SET tx_sig = EXCLUDED.tx_sig, status = 'APPROVED', updated_at = NOW()"#,
    )
    .bind(row.id)
    .bind(wallet.to_string())
    .bind(sig.to_string())
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    let (row, proposal) = sync_proposal(&state, &row).await?;
    let stale = sqlx::query_scalar::<_, String>(
        "SELECT stale_transaction_index::text FROM settings_accounts WHERE id = $1",
    )
    .bind(row.settings_account_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?
    .parse::<u64>()
    .map_err(internal_error)?;
    Ok(Json(TransactionProposalEnvelope {
        transaction: transaction_response(&state, row, stale).await?,
        proposal: proposal_response(&proposal),
    }))
}

pub async fn build_execute_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WalletQuery>,
) -> Result<Json<BuiltActionResponse>, ApiError> {
    build_execute_transaction_inner(state, id, query).await
}

pub async fn build_reject_transaction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WalletQuery>,
) -> Result<Json<BuiltActionResponse>, ApiError> {
    let wallet = parse_pubkey(
        query.wallet_address.as_deref().unwrap_or(""),
        "walletAddress",
    )?;
    let row = load_transaction(&state, &id).await?;
    let settings = sqlx::query_as::<_, SettingsForTransaction>(r#"SELECT id, workspace_id, pda, stale_transaction_index::text AS stale_transaction_index FROM settings_accounts WHERE id = $1"#).bind(row.settings_account_id).fetch_one(&state.db).await.map_err(internal_error)?;
    require_permission(&state, settings.id, &wallet, VOTE_PERMISSION).await?;
    let stale = settings
        .stale_transaction_index
        .parse::<u64>()
        .map_err(internal_error)?;
    let index = row
        .transaction_index
        .parse::<u64>()
        .map_err(internal_error)?;
    if row.status == "PROPOSAL_ACTIVE" && index <= stale {
        return Err(bad_request(
            "This proposal became stale after the signer or threshold settings changed. Create a new transaction proposal.",
        ));
    }
    if row.status != "PROPOSAL_ACTIVE" {
        return Err(bad_request(format!(
            "A {} proposal cannot be rejected",
            row.status.to_lowercase()
        )));
    }
    let settings_pda = parse_pubkey(&settings.pda, "settingsPda")?;
    let (proposal_pda, instruction) =
        build_reject_proposal_instruction(settings_pda, wallet, index).map_err(internal_error)?;
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    Ok(Json(BuiltActionResponse {
        transaction_id: row.id,
        wallet_address: wallet.to_string(),
        transaction_base64: build_unsigned_transaction_base64(wallet, blockhash, instruction)
            .map_err(internal_error)?,
        proposal_pda: proposal_pda.to_string(),
        smart_account_pda: None,
    }))
}

pub async fn rejection_submitted(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SubmittedActionRequest>,
) -> Result<Json<TransactionProposalEnvelope>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let sig = parse_signature(&payload.tx_sig, "txSig")?;
    let row = load_transaction(&state, &id).await?;
    let settings_pda = parse_pubkey(
        &sqlx::query_scalar::<_, String>("SELECT pda FROM settings_accounts WHERE id = $1")
            .bind(row.settings_account_id)
            .fetch_one(&state.db)
            .await
            .map_err(internal_error)?,
        "settingsPda",
    )?;
    let proposal_pda = parse_pubkey(&row.proposal_pda, "proposalPda")?;
    verify_confirmed_squads_instruction(
        &state.rpc,
        &sig,
        &wallet,
        &[settings_pda, proposal_pda],
        "global:reject_proposal",
    )
    .await
    .map_err(bad_request)?;
    let proposal = fetch_proposal(&state, &proposal_pda).await?;
    if !proposal.rejected.contains(&wallet) {
        return Err(bad_request(
            "Confirmed transaction did not record the requested rejection",
        ));
    }
    sqlx::query(
        r#"INSERT INTO proposal_approvals (transaction_record_id, wallet_address, tx_sig, status)
        VALUES ($1,$2,$3,'REJECTED') ON CONFLICT (transaction_record_id, wallet_address)
        DO UPDATE SET tx_sig = EXCLUDED.tx_sig, status = 'REJECTED', updated_at = NOW()"#,
    )
    .bind(row.id)
    .bind(wallet.to_string())
    .bind(sig.to_string())
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    let (row, proposal) = sync_proposal(&state, &row).await?;
    let stale = sqlx::query_scalar::<_, String>(
        "SELECT stale_transaction_index::text FROM settings_accounts WHERE id = $1",
    )
    .bind(row.settings_account_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?
    .parse::<u64>()
    .map_err(internal_error)?;
    Ok(Json(TransactionProposalEnvelope {
        transaction: transaction_response(&state, row, stale).await?,
        proposal: proposal_response(&proposal),
    }))
}

async fn build_execute_transaction_inner(
    state: AppState,
    id: String,
    query: WalletQuery,
) -> Result<Json<BuiltActionResponse>, ApiError> {
    let wallet = parse_pubkey(
        query.wallet_address.as_deref().unwrap_or(""),
        "walletAddress",
    )?;
    let row = load_transaction(&state, &id).await?;
    if row.status != "APPROVED" {
        return Err(bad_request("Only an approved proposal can be executed"));
    }
    let settings_pda_text =
        sqlx::query_scalar::<_, String>("SELECT pda FROM settings_accounts WHERE id = $1")
            .bind(row.settings_account_id)
            .fetch_one(&state.db)
            .await
            .map_err(internal_error)?;
    require_permission(&state, row.settings_account_id, &wallet, EXECUTE_PERMISSION).await?;
    let settings_pda = parse_pubkey(&settings_pda_text, "settingsPda")?;
    let transaction_pda = parse_pubkey(&row.transaction_pda, "transactionPda")?;
    let transaction_account = state
        .rpc
        .get_account(&transaction_pda)
        .await
        .map_err(internal_error)?;
    let message = decode_transaction_message(&transaction_account.owner, &transaction_account.data)
        .map_err(internal_error)?;
    let (proposal_pda, _, instruction) = build_execute_transaction_instruction(
        settings_pda,
        wallet,
        row.transaction_index
            .parse::<u64>()
            .map_err(internal_error)?,
        &message,
    )
    .map_err(internal_error)?;
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    Ok(Json(BuiltActionResponse {
        transaction_id: row.id,
        wallet_address: wallet.to_string(),
        transaction_base64: build_unsigned_transaction_base64(wallet, blockhash, instruction)
            .map_err(internal_error)?,
        proposal_pda: proposal_pda.to_string(),
        smart_account_pda: Some(row.smart_account_pda),
    }))
}

pub async fn execute_submitted(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SubmittedActionRequest>,
) -> Result<Json<TransactionProposalEnvelope>, ApiError> {
    let wallet = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let sig = parse_signature(&payload.tx_sig, "txSig")?;
    let row = load_transaction(&state, &id).await?;
    let settings_pda = parse_pubkey(
        &sqlx::query_scalar::<_, String>("SELECT pda FROM settings_accounts WHERE id = $1")
            .bind(row.settings_account_id)
            .fetch_one(&state.db)
            .await
            .map_err(internal_error)?,
        "settingsPda",
    )?;
    let transaction_pda = parse_pubkey(&row.transaction_pda, "transactionPda")?;
    let proposal_pda = parse_pubkey(&row.proposal_pda, "proposalPda")?;
    verify_confirmed_squads_instruction(
        &state.rpc,
        &sig,
        &wallet,
        &[settings_pda, transaction_pda, proposal_pda],
        "global:execute_transaction",
    )
    .await
    .map_err(bad_request)?;
    let proposal = fetch_proposal(&state, &proposal_pda).await?;
    if proposal.status.database_value() != "EXECUTED" {
        return Err(bad_request(
            "Confirmed transaction did not execute the proposal",
        ));
    }
    let row = sqlx::query_as::<_, TransactionRow>(
        r#"UPDATE transaction_records
           SET status = 'EXECUTED', execute_tx_sig = $2, updated_at = NOW()
           WHERE id = $1
           RETURNING id, settings_account_id, smart_account_id, transaction_pda,
                     proposal_pda, smart_account_pda, transaction_index::text AS transaction_index,
                     account_index, recipient, amount_lamports::text AS amount_lamports, memo,
                     status, create_tx_sig, proposal_tx_sig, execute_tx_sig, created_by_wallet,
                     created_at, updated_at"#,
    )
    .bind(row.id)
    .bind(sig.to_string())
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    mark_transaction_receipt_paid(&state, row.id).await?;
    let stale = sqlx::query_scalar::<_, String>(
        "SELECT stale_transaction_index::text FROM settings_accounts WHERE id = $1",
    )
    .bind(row.settings_account_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?
    .parse::<u64>()
    .map_err(internal_error)?;
    Ok(Json(TransactionProposalEnvelope {
        transaction: transaction_response(&state, row, stale).await?,
        proposal: proposal_response(&proposal),
    }))
}
use std::collections::HashMap;
