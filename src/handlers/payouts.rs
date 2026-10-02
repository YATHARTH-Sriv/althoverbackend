use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    ApiError, AppState,
    access::{SettingsAccess, require_settings_access},
    bad_request, internal_error,
    validation::{optional_text, required_text},
};

use super::auth::normalize_wallet_address;

const INITIATE_PERMISSION: i32 = 1;
const CATEGORIES: [&str; 7] = [
    "PAYROLL",
    "GRANT",
    "BOUNTY",
    "CONTRACTOR",
    "VENDOR",
    "REIMBURSEMENT",
    "OTHER",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayoutRecipientInput {
    name: String,
    wallet_address: String,
    amount_lamports: String,
    memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePayoutRequest {
    wallet_address: String,
    settings_pda: String,
    smart_account_id: Uuid,
    title: String,
    description: Option<String>,
    category: String,
    recipients: Vec<PayoutRecipientInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePayoutRequest {
    wallet_address: String,
    settings_pda: String,
    smart_account_id: Option<Uuid>,
    title: Option<String>,
    description: Option<String>,
    category: Option<String>,
    recipients: Option<Vec<PayoutRecipientInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayoutQuery {
    wallet_address: String,
    settings_pda: String,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTransactionRequest {
    wallet_address: String,
    settings_pda: String,
    transaction_record_id: Uuid,
}

#[derive(Debug, FromRow)]
struct PayoutRow {
    id: Uuid,
    workspace_id: Uuid,
    smart_account_id: Uuid,
    smart_account_name: String,
    account_index: i32,
    title: String,
    description: Option<String>,
    category: String,
    stored_status: String,
    created_by_wallet: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayoutRecipientResponse {
    id: Uuid,
    position: i32,
    name: String,
    wallet_address: String,
    amount_lamports: String,
    memo: Option<String>,
    transaction_record_id: Option<Uuid>,
    transaction_status: Option<String>,
    transaction_index: Option<String>,
    proposal_pda: Option<String>,
    execute_tx_sig: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayoutResponse {
    id: Uuid,
    workspace_id: Uuid,
    smart_account_id: Uuid,
    smart_account_name: String,
    account_index: i32,
    title: String,
    description: Option<String>,
    category: String,
    status: String,
    total_lamports: String,
    created_by_wallet: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    recipients: Vec<PayoutRecipientResponse>,
}

#[derive(Debug, Serialize)]
pub struct PayoutItemResponse {
    payout: PayoutResponse,
}

#[derive(Debug, Serialize)]
pub struct PayoutListResponse {
    payouts: Vec<PayoutResponse>,
}

fn category(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_uppercase();
    CATEGORIES
        .contains(&value.as_str())
        .then_some(value)
        .ok_or_else(|| bad_request("category is invalid"))
}

async fn payout_access(
    state: &AppState,
    settings_pda: &str,
    wallet_address: &str,
    require_initiate: bool,
) -> Result<SettingsAccess, ApiError> {
    require_settings_access(
        state,
        settings_pda,
        wallet_address,
        require_initiate.then_some(INITIATE_PERMISSION),
        false,
    )
    .await
}

async fn validate_smart_account(
    transaction: &mut Transaction<'_, Postgres>,
    access: &SettingsAccess,
    smart_account_id: Uuid,
) -> Result<(), ApiError> {
    let valid = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM smart_accounts WHERE id=$1 AND settings_account_id=$2)",
    )
    .bind(smart_account_id)
    .bind(access.settings_account_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(internal_error)?;
    if !valid {
        return Err(bad_request("Smart account was not found in this workspace"));
    }
    Ok(())
}

async fn replace_recipients(
    transaction: &mut Transaction<'_, Postgres>,
    payout_id: Uuid,
    recipients: Vec<PayoutRecipientInput>,
) -> Result<(), ApiError> {
    if recipients.is_empty() {
        return Err(bad_request("Add at least one payout recipient"));
    }
    if recipients.len() > 100 {
        return Err(bad_request("A payout can contain at most 100 recipients"));
    }
    sqlx::query("DELETE FROM payout_recipients WHERE payout_batch_id=$1")
        .bind(payout_id)
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
    for (position, recipient) in recipients.into_iter().enumerate() {
        let name = required_text(&recipient.name, "recipient name", 120)?;
        let wallet = Pubkey::from_str(recipient.wallet_address.trim())
            .map_err(|_| bad_request("recipient walletAddress is invalid"))?
            .to_string();
        let amount = recipient
            .amount_lamports
            .trim()
            .parse::<u64>()
            .map_err(|_| bad_request("amountLamports must be a positive integer"))?;
        if amount == 0 {
            return Err(bad_request("amountLamports must be greater than zero"));
        }
        let memo = optional_text(recipient.memo, "memo", 240)?;
        sqlx::query(
            r#"INSERT INTO payout_recipients
               (payout_batch_id, position, name, wallet_address, amount_lamports, memo)
               VALUES ($1,$2,$3,$4,$5::numeric,$6)"#,
        )
        .bind(payout_id)
        .bind(position as i32)
        .bind(name)
        .bind(wallet)
        .bind(amount.to_string())
        .bind(memo)
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
    }
    Ok(())
}

fn derived_status(stored: &str, recipients: &[PayoutRecipientResponse]) -> String {
    if stored == "CANCELLED" {
        return stored.to_owned();
    }
    let linked = recipients
        .iter()
        .filter(|recipient| recipient.transaction_record_id.is_some())
        .count();
    if linked == 0 {
        return "DRAFT".to_owned();
    }
    let executed = recipients
        .iter()
        .filter(|recipient| recipient.transaction_status.as_deref() == Some("EXECUTED"))
        .count();
    let failed = recipients
        .iter()
        .filter(|recipient| {
            matches!(
                recipient.transaction_status.as_deref(),
                Some("REJECTED" | "CANCELLED")
            )
        })
        .count();
    if executed == recipients.len() {
        "COMPLETED".to_owned()
    } else if failed == recipients.len() {
        "CANCELLED".to_owned()
    } else if linked == recipients.len() && executed + failed == recipients.len() && failed > 0 {
        "PARTIALLY_FAILED".to_owned()
    } else {
        "IN_PROGRESS".to_owned()
    }
}

async fn load_payout(
    state: &AppState,
    workspace_id: Uuid,
    payout_id: Uuid,
) -> Result<PayoutResponse, ApiError> {
    let row = sqlx::query_as::<_, PayoutRow>(
        r#"SELECT p.id, p.workspace_id, p.smart_account_id, sa.name AS smart_account_name,
                  sa.account_index, p.title, p.description, p.category,
                  p.status AS stored_status, p.created_by_wallet, p.created_at, p.updated_at
           FROM payout_batches p JOIN smart_accounts sa ON sa.id=p.smart_account_id
           WHERE p.id=$1 AND p.workspace_id=$2"#,
    )
    .bind(payout_id)
    .bind(workspace_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Payout not found"))?;
    let recipients = sqlx::query_as::<_, PayoutRecipientResponse>(
        r#"SELECT pr.id, pr.position, pr.name, pr.wallet_address,
                  pr.amount_lamports::text AS amount_lamports, pr.memo,
                  pr.transaction_record_id, tr.status AS transaction_status,
                  tr.transaction_index::text AS transaction_index,
                  tr.proposal_pda, tr.execute_tx_sig
           FROM payout_recipients pr
           LEFT JOIN transaction_records tr ON tr.id=pr.transaction_record_id
           WHERE pr.payout_batch_id=$1 ORDER BY pr.position"#,
    )
    .bind(payout_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let total = recipients.iter().try_fold(0_u128, |sum, recipient| {
        recipient
            .amount_lamports
            .parse::<u128>()
            .ok()
            .and_then(|amount| sum.checked_add(amount))
    });
    let total = total.ok_or_else(|| internal_error("Payout total overflow"))?;
    Ok(PayoutResponse {
        id: row.id,
        workspace_id: row.workspace_id,
        smart_account_id: row.smart_account_id,
        smart_account_name: row.smart_account_name,
        account_index: row.account_index,
        title: row.title,
        description: row.description,
        category: row.category,
        status: derived_status(&row.stored_status, &recipients),
        total_lamports: total.to_string(),
        created_by_wallet: row.created_by_wallet,
        created_at: row.created_at,
        updated_at: row.updated_at,
        recipients,
    })
}

pub async fn create_payout(
    State(state): State<AppState>,
    Json(body): Json<CreatePayoutRequest>,
) -> Result<(StatusCode, Json<PayoutItemResponse>), ApiError> {
    let access = payout_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let wallet = normalize_wallet_address(&body.wallet_address)?;
    let title = required_text(&body.title, "title", 160)?;
    let description = optional_text(body.description, "description", 1000)?;
    let category = category(&body.category)?;
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    validate_smart_account(&mut transaction, &access, body.smart_account_id).await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO payout_batches
           (workspace_id, smart_account_id, title, description, category, created_by_wallet)
           VALUES ($1,$2,$3,$4,$5,$6) RETURNING id"#,
    )
    .bind(access.workspace_id)
    .bind(body.smart_account_id)
    .bind(title)
    .bind(description)
    .bind(category)
    .bind(wallet)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    replace_recipients(&mut transaction, id, body.recipients).await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok((
        StatusCode::CREATED,
        Json(PayoutItemResponse {
            payout: load_payout(&state, access.workspace_id, id).await?,
        }),
    ))
}

pub async fn list_payouts(
    State(state): State<AppState>,
    Query(query): Query<PayoutQuery>,
) -> Result<Json<PayoutListResponse>, ApiError> {
    let access = payout_access(&state, &query.settings_pda, &query.wallet_address, false).await?;
    let (limit, offset) = crate::pagination::bounds(query.limit, query.offset);
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM payout_batches WHERE workspace_id=$1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(access.workspace_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let mut payouts = Vec::with_capacity(ids.len());
    for id in ids {
        payouts.push(load_payout(&state, access.workspace_id, id).await?);
    }
    Ok(Json(PayoutListResponse { payouts }))
}

pub async fn get_payout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<PayoutQuery>,
) -> Result<Json<PayoutItemResponse>, ApiError> {
    let access = payout_access(&state, &query.settings_pda, &query.wallet_address, false).await?;
    Ok(Json(PayoutItemResponse {
        payout: load_payout(&state, access.workspace_id, id).await?,
    }))
}

pub async fn update_payout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePayoutRequest>,
) -> Result<Json<PayoutItemResponse>, ApiError> {
    let access = payout_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let current = load_payout(&state, access.workspace_id, id).await?;
    if current.status != "DRAFT" {
        return Err(bad_request("Only draft payouts can be edited"));
    }
    let smart_account_id = body.smart_account_id.unwrap_or(current.smart_account_id);
    let title = body
        .title
        .map(|value| required_text(&value, "title", 160))
        .transpose()?
        .unwrap_or(current.title);
    let description = match body.description {
        Some(value) => optional_text(Some(value), "description", 1000)?,
        None => current.description,
    };
    let category = body
        .category
        .map(|value| category(&value))
        .transpose()?
        .unwrap_or(current.category);
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    validate_smart_account(&mut transaction, &access, smart_account_id).await?;
    sqlx::query(
        "UPDATE payout_batches SET smart_account_id=$1,title=$2,description=$3,category=$4,updated_at=NOW() WHERE id=$5 AND workspace_id=$6",
    )
    .bind(smart_account_id)
    .bind(title)
    .bind(description)
    .bind(category)
    .bind(id)
    .bind(access.workspace_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if let Some(recipients) = body.recipients {
        replace_recipients(&mut transaction, id, recipients).await?;
    }
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(PayoutItemResponse {
        payout: load_payout(&state, access.workspace_id, id).await?,
    }))
}

pub async fn delete_payout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PayoutQuery>,
) -> Result<StatusCode, ApiError> {
    let access = payout_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let result = sqlx::query(
        "DELETE FROM payout_batches WHERE id=$1 AND workspace_id=$2 AND status='DRAFT'",
    )
    .bind(id)
    .bind(access.workspace_id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(bad_request("Only an existing draft payout can be deleted"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn link_payout_transaction(
    State(state): State<AppState>,
    Path((id, recipient_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<LinkTransactionRequest>,
) -> Result<Json<PayoutItemResponse>, ApiError> {
    let access = payout_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let linked = sqlx::query(
        r#"UPDATE payout_recipients pr SET transaction_record_id=$1, updated_at=NOW()
           FROM payout_batches p, transaction_records tr
           WHERE pr.id=$2 AND pr.payout_batch_id=$3 AND p.id=pr.payout_batch_id
             AND p.workspace_id=$4 AND tr.id=$1 AND tr.smart_account_id=p.smart_account_id
             AND tr.recipient=pr.wallet_address
             AND tr.amount_lamports=pr.amount_lamports"#,
    )
    .bind(body.transaction_record_id)
    .bind(recipient_id)
    .bind(id)
    .bind(access.workspace_id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    if linked.rows_affected() == 0 {
        return Err(bad_request(
            "Transaction does not match this payout recipient",
        ));
    }
    sqlx::query("UPDATE payout_batches SET status='IN_PROGRESS',updated_at=NOW() WHERE id=$1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    Ok(Json(PayoutItemResponse {
        payout: load_payout(&state, access.workspace_id, id).await?,
    }))
}
