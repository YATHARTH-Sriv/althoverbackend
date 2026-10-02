use axum::{
    Json,
    extract::{Path, Query, State},
};
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        build_unsigned_transaction_base64, derive_proposal_pda, derive_smart_account_pda,
        derive_transaction_pda, verify_confirmed_squads_instruction,
    },
    squadsaccounts::{
        build_approve_proposal_instruction, build_create_proposal_instruction,
        build_create_transaction_instruction, build_execute_transaction_instruction,
        build_reject_proposal_instruction, decode_settings_account, decode_transaction_message,
    },
    validation::{parse_pubkey, parse_signature},
};

use super::super::receipts::{link_receipt_to_transaction, mark_transaction_receipt_paid};
use super::models::*;
use super::repository::*;
use super::service::*;

const INITIATE_PERMISSION: i32 = 1;
const VOTE_PERMISSION: i32 = 2;
const EXECUTE_PERMISSION: i32 = 4;

fn normalize_memo(value: Option<String>) -> String {
    value
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_owned()))
        .unwrap_or_else(|| "Hover treasury payment".to_owned())
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
