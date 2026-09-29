use std::str::FromStr;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::settings_management::refresh_settings_by_pda;
use super::smart_accounts::{SmartAccountResponse, SmartAccountRow, refresh_smart_account_balance};
use super::transaction_lifecycle::{TransactionResponse, load_transactions_for_dashboard};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardQuery {
    wallet_address: Option<String>,
    refresh: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardResponse {
    wallet_address: Option<String>,
    settings: Vec<DashboardSettingsResponse>,
}

#[derive(Debug, FromRow)]
struct DashboardSettingsRow {
    id: Uuid,
    name: String,
    pda: String,
    settings_authority: String,
    threshold: i32,
    time_lock: i64,
    transaction_index: String,
    stale_transaction_index: String,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardSignerResponse {
    id: Uuid,
    role: String,
    label: Option<String>,
    name: Option<String>,
    email: Option<String>,
    designation: Option<String>,
    wallet_label: Option<String>,
    wallet_address: String,
    permissions_mask: i32,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardActivityResponse {
    id: Uuid,
    #[serde(rename = "type")]
    activity_type: String,
    title: String,
    description: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSettingsResponse {
    id: Uuid,
    name: String,
    pda: String,
    settings_authority: String,
    threshold: i32,
    time_lock: i64,
    transaction_index: String,
    stale_transaction_index: String,
    signers: Vec<DashboardSignerResponse>,
    smart_accounts: Vec<SmartAccountResponse>,
    transactions: Vec<TransactionResponse>,
    receipts: Vec<serde_json::Value>,
    activities: Vec<DashboardActivityResponse>,
}

fn parse_wallet_address(value: &str) -> Result<Pubkey, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request("walletAddress is required"));
    }

    Pubkey::from_str(value).map_err(|_| bad_request("walletAddress is invalid"))
}

async fn load_settings_rows(
    state: &AppState,
    wallet_address: &Pubkey,
) -> Result<Vec<DashboardSettingsRow>, ApiError> {
    sqlx::query_as::<_, DashboardSettingsRow>(
        r#"
        SELECT
            settings.id,
            settings.name,
            settings.pda,
            settings.settings_authority,
            settings.threshold,
            settings.time_lock,
            settings.transaction_index::text AS transaction_index,
            settings.stale_transaction_index::text AS stale_transaction_index
        FROM settings_accounts AS settings
        WHERE EXISTS (
            SELECT 1
            FROM settings_signers AS signer
            WHERE signer.settings_account_id = settings.id
              AND signer.wallet_address = $1
        )
        ORDER BY settings.created_at DESC
        "#,
    )
    .bind(wallet_address.to_string())
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)
}

async fn load_signers(
    state: &AppState,
    settings_id: Uuid,
) -> Result<Vec<DashboardSignerResponse>, ApiError> {
    sqlx::query_as::<_, DashboardSignerResponse>(
        r#"
        SELECT
            signer.id,
            signer.role,
            signer.label,
            app_user.name,
            app_user.email,
            app_user.designation,
            wallet.label AS wallet_label,
            signer.wallet_address,
            signer.permissions_mask
        FROM settings_signers AS signer
        LEFT JOIN wallets AS wallet
            ON wallet.address = signer.wallet_address
        LEFT JOIN users AS app_user
            ON app_user.id = wallet.user_id
        WHERE signer.settings_account_id = $1
        ORDER BY signer.created_at ASC
        "#,
    )
    .bind(settings_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)
}

async fn load_activities(
    state: &AppState,
    settings_id: Uuid,
) -> Result<Vec<DashboardActivityResponse>, ApiError> {
    sqlx::query_as::<_, DashboardActivityResponse>(
        r#"
        SELECT id, activity_type, title, description, created_at
        FROM activity_logs
        WHERE settings_account_id = $1
        ORDER BY created_at DESC
        LIMIT 20
        "#,
    )
    .bind(settings_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)
}

async fn load_smart_accounts(
    state: &AppState,
    settings_id: Uuid,
) -> Result<Vec<SmartAccountRow>, ApiError> {
    sqlx::query_as::<_, SmartAccountRow>(
        r#"
        SELECT
            id,
            settings_account_id,
            name,
            account_index,
            pda,
            balance_lamports::text AS balance_lamports,
            created_by_wallet,
            created_at,
            updated_at
        FROM smart_accounts
        WHERE settings_account_id = $1
        ORDER BY account_index ASC
        "#,
    )
    .bind(settings_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)
}

pub async fn dashboard(
    State(state): State<AppState>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<DashboardResponse>, ApiError> {
    let Some(wallet_address_raw) = query.wallet_address else {
        return Ok(Json(DashboardResponse {
            wallet_address: None,
            settings: Vec::new(),
        }));
    };

    let wallet_address = parse_wallet_address(&wallet_address_raw)?;
    let mut settings_rows = load_settings_rows(&state, &wallet_address).await?;

    if query.refresh.unwrap_or(false) {
        for settings in &settings_rows {
            let settings_pda = Pubkey::from_str(&settings.pda)
                .map_err(|_| internal_error("Indexed settings PDA is invalid"))?;
            refresh_settings_by_pda(&state, &settings_pda).await?;
        }

        settings_rows = load_settings_rows(&state, &wallet_address).await?;
    }

    let mut settings = Vec::with_capacity(settings_rows.len());

    for row in settings_rows {
        let signers = load_signers(&state, row.id).await?;
        let activities = load_activities(&state, row.id).await?;
        let mut smart_account_rows = load_smart_accounts(&state, row.id).await?;

        if query.refresh.unwrap_or(false) {
            let mut refreshed_rows = Vec::with_capacity(smart_account_rows.len());
            for smart_account in &smart_account_rows {
                refreshed_rows.push(refresh_smart_account_balance(&state, smart_account).await?);
            }
            smart_account_rows = refreshed_rows;
        }

        let smart_accounts = smart_account_rows
            .into_iter()
            .map(SmartAccountResponse::from)
            .collect();
        let stale_index = row
            .stale_transaction_index
            .parse::<u64>()
            .map_err(internal_error)?;
        let transactions = load_transactions_for_dashboard(
            &state,
            row.id,
            stale_index,
            query.refresh.unwrap_or(false),
        )
        .await?;

        settings.push(DashboardSettingsResponse {
            id: row.id,
            name: row.name,
            pda: row.pda,
            settings_authority: row.settings_authority,
            threshold: row.threshold,
            time_lock: row.time_lock,
            transaction_index: row.transaction_index,
            stale_transaction_index: row.stale_transaction_index,
            signers,
            smart_accounts,
            transactions,
            receipts: Vec::new(),
            activities,
        });
    }

    Ok(Json(DashboardResponse {
        wallet_address: Some(wallet_address.to_string()),
        settings,
    }))
}
