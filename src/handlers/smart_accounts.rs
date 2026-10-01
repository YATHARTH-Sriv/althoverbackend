use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use solana_system_interface::instruction as system_instruction;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        build_unsigned_transaction_base64, derive_smart_account_pda, verify_confirmed_transaction,
    },
    validation::parse_pubkey,
};

#[derive(Debug, FromRow)]
struct SettingsOwnerRow {
    id: Uuid,
    workspace_id: Uuid,
    created_by_wallet: String,
    settings_authority: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct SmartAccountRow {
    pub(crate) id: Uuid,
    pub(crate) settings_account_id: Uuid,
    pub(crate) name: String,
    pub(crate) account_index: i32,
    pub(crate) pda: String,
    pub(crate) balance_lamports: String,
    pub(crate) created_by_wallet: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SmartAccountResponse {
    pub(crate) id: Uuid,
    pub(crate) settings_account_id: Uuid,
    pub(crate) name: String,
    pub(crate) account_index: i32,
    pub(crate) pda: String,
    pub(crate) balance_lamports: String,
    pub(crate) created_by_wallet: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<SmartAccountRow> for SmartAccountResponse {
    fn from(row: SmartAccountRow) -> Self {
        Self {
            id: row.id,
            settings_account_id: row.settings_account_id,
            name: row.name,
            account_index: row.account_index,
            pda: row.pda,
            balance_lamports: row.balance_lamports,
            created_by_wallet: row.created_by_wallet,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSmartAccountRequest {
    wallet_address: String,
    settings_pda: String,
    name: Option<String>,
    account_index: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSmartAccountResponse {
    smart_account: SmartAccountResponse,
    bump: u8,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LamportAmount {
    Text(String),
    Number(u64),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildFundSmartAccountRequest {
    wallet_address: String,
    smart_account_pda: String,
    amount_lamports: LamportAmount,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundSmartAccountSubmittedRequest {
    wallet_address: String,
    smart_account_pda: String,
    amount_lamports: LamportAmount,
    tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildFundSmartAccountResponse {
    wallet_address: String,
    smart_account_id: Uuid,
    smart_account_name: String,
    smart_account_pda: String,
    settings_pda: String,
    amount_lamports: String,
    transaction_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartAccountEnvelope {
    smart_account: SmartAccountResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FundSmartAccountSubmittedResponse {
    smart_account: SmartAccountResponse,
    tx_sig: String,
}

fn parse_signature(value: &str) -> Result<Signature, ApiError> {
    Signature::from_str(value.trim()).map_err(|_| bad_request("txSig is invalid"))
}

fn parse_amount(value: LamportAmount) -> Result<u64, ApiError> {
    let amount = match value {
        LamportAmount::Text(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|_| bad_request("amountLamports must be a positive integer"))?,
        LamportAmount::Number(value) => value,
    };

    if amount == 0 {
        return Err(bad_request("Funding amount must be greater than 0"));
    }

    Ok(amount)
}

fn normalize_name(value: Option<String>, account_index: u8) -> String {
    value
        .and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        })
        .unwrap_or_else(|| format!("Treasury Index_{account_index}"))
}

async fn load_smart_account_by_pda(
    state: &AppState,
    pda: &Pubkey,
) -> Result<(SmartAccountRow, String, Uuid), ApiError> {
    #[derive(FromRow)]
    struct JoinedRow {
        id: Uuid,
        settings_account_id: Uuid,
        name: String,
        account_index: i32,
        pda: String,
        balance_lamports: String,
        created_by_wallet: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        settings_pda: String,
        workspace_id: Uuid,
    }

    let row = sqlx::query_as::<_, JoinedRow>(
        r#"
        SELECT
            smart.id,
            smart.settings_account_id,
            smart.name,
            smart.account_index,
            smart.pda,
            smart.balance_lamports::text AS balance_lamports,
            smart.created_by_wallet,
            smart.created_at,
            smart.updated_at,
            settings.pda AS settings_pda,
            settings.workspace_id
        FROM smart_accounts AS smart
        JOIN settings_accounts AS settings
            ON settings.id = smart.settings_account_id
        WHERE smart.pda = $1
        "#,
    )
    .bind(pda.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Smart account is not indexed yet"))?;

    let settings_pda = parse_pubkey(&row.settings_pda, "indexed settings PDA")?;
    let account_index = u8::try_from(row.account_index)
        .map_err(|_| internal_error("Indexed smart account has an invalid account index"))?;

    if derive_smart_account_pda(&settings_pda, account_index).0 != *pda {
        return Err(internal_error(
            "Indexed smart account PDA does not match its settings account and index",
        ));
    }

    Ok((
        SmartAccountRow {
            id: row.id,
            settings_account_id: row.settings_account_id,
            name: row.name,
            account_index: row.account_index,
            pda: row.pda,
            balance_lamports: row.balance_lamports,
            created_by_wallet: row.created_by_wallet,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
        row.settings_pda,
        row.workspace_id,
    ))
}

pub(crate) async fn refresh_smart_account_balance(
    state: &AppState,
    smart_account: &SmartAccountRow,
) -> Result<SmartAccountRow, ApiError> {
    let pda = parse_pubkey(&smart_account.pda, "smartAccountPda")?;
    let balance = state.rpc.get_balance(&pda).await.map_err(internal_error)?;

    sqlx::query_as::<_, SmartAccountRow>(
        r#"
        UPDATE smart_accounts
        SET balance_lamports = $2::numeric, updated_at = NOW()
        WHERE id = $1
        RETURNING
            id,
            settings_account_id,
            name,
            account_index,
            pda,
            balance_lamports::text AS balance_lamports,
            created_by_wallet,
            created_at,
            updated_at
        "#,
    )
    .bind(smart_account.id)
    .bind(balance.to_string())
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)
}

pub async fn create_smart_account(
    State(state): State<AppState>,
    Json(payload): Json<CreateSmartAccountRequest>,
) -> Result<Json<CreateSmartAccountResponse>, ApiError> {
    let wallet_address = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let account_index = u8::try_from(payload.account_index)
        .map_err(|_| bad_request("accountIndex must be an integer between 0 and 255"))?;
    let name = normalize_name(payload.name, account_index);

    let settings = sqlx::query_as::<_, SettingsOwnerRow>(
        r#"
        SELECT id, workspace_id, created_by_wallet, settings_authority
        FROM settings_accounts
        WHERE pda = $1
        "#,
    )
    .bind(settings_pda.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account is not indexed yet"))?;

    let wallet = wallet_address.to_string();
    if settings.created_by_wallet != wallet && settings.settings_authority != wallet {
        return Err(bad_request(
            "Only the settings owner can create smart account records",
        ));
    }

    let (smart_account_pda, bump) = derive_smart_account_pda(&settings_pda, account_index);
    let balance = state
        .rpc
        .get_balance(&smart_account_pda)
        .await
        .map_err(internal_error)?;
    let mut database_transaction = state.db.begin().await.map_err(internal_error)?;

    let smart_account = sqlx::query_as::<_, SmartAccountRow>(
        r#"
        INSERT INTO smart_accounts (
            settings_account_id,
            name,
            account_index,
            pda,
            balance_lamports,
            created_by_wallet
        )
        VALUES ($1, $2, $3, $4, $5::numeric, $6)
        ON CONFLICT (settings_account_id, account_index)
        DO UPDATE SET
            name = EXCLUDED.name,
            balance_lamports = EXCLUDED.balance_lamports,
            updated_at = NOW()
        RETURNING
            id,
            settings_account_id,
            name,
            account_index,
            pda,
            balance_lamports::text AS balance_lamports,
            created_by_wallet,
            created_at,
            updated_at
        "#,
    )
    .bind(settings.id)
    .bind(name)
    .bind(i32::from(account_index))
    .bind(smart_account_pda.to_string())
    .bind(balance.to_string())
    .bind(&wallet)
    .fetch_one(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    sqlx::query(
        r#"
        INSERT INTO activity_logs (
            workspace_id,
            settings_account_id,
            smart_account_id,
            activity_type,
            title,
            metadata
        )
        VALUES ($1, $2, $3, 'SMART_ACCOUNT_CREATED',
                'Smart account record created', $4)
        "#,
    )
    .bind(settings.workspace_id)
    .bind(settings.id)
    .bind(smart_account.id)
    .bind(serde_json::json!({
        "settingsPda": settings_pda.to_string(),
        "smartAccountPda": smart_account_pda.to_string(),
        "accountIndex": account_index,
        "bump": bump
    }))
    .execute(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    database_transaction
        .commit()
        .await
        .map_err(internal_error)?;

    Ok(Json(CreateSmartAccountResponse {
        smart_account: smart_account.into(),
        bump,
    }))
}

pub async fn build_fund_smart_account(
    State(state): State<AppState>,
    Json(payload): Json<BuildFundSmartAccountRequest>,
) -> Result<Json<BuildFundSmartAccountResponse>, ApiError> {
    let wallet_address = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let smart_account_pda = parse_pubkey(&payload.smart_account_pda, "smartAccountPda")?;
    let amount_lamports = parse_amount(payload.amount_lamports)?;
    let (smart_account, settings_pda, _) =
        load_smart_account_by_pda(&state, &smart_account_pda).await?;

    let instruction =
        system_instruction::transfer(&wallet_address, &smart_account_pda, amount_lamports);
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    let transaction_base64 =
        build_unsigned_transaction_base64(wallet_address, blockhash, instruction)
            .map_err(internal_error)?;

    Ok(Json(BuildFundSmartAccountResponse {
        wallet_address: wallet_address.to_string(),
        smart_account_id: smart_account.id,
        smart_account_name: smart_account.name,
        smart_account_pda: smart_account.pda,
        settings_pda,
        amount_lamports: amount_lamports.to_string(),
        transaction_base64,
    }))
}

pub async fn fund_smart_account_submitted(
    State(state): State<AppState>,
    Json(payload): Json<FundSmartAccountSubmittedRequest>,
) -> Result<Json<FundSmartAccountSubmittedResponse>, ApiError> {
    let wallet_address = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let smart_account_pda = parse_pubkey(&payload.smart_account_pda, "smartAccountPda")?;
    let amount_lamports = parse_amount(payload.amount_lamports)?;
    let signature = parse_signature(&payload.tx_sig)?;
    let (smart_account, _, workspace_id) =
        load_smart_account_by_pda(&state, &smart_account_pda).await?;

    verify_confirmed_transaction(
        state.rpc.as_ref(),
        &signature,
        &wallet_address,
        &[smart_account_pda],
    )
    .await
    .map_err(bad_request)?;

    let refreshed = refresh_smart_account_balance(&state, &smart_account).await?;

    sqlx::query(
        r#"
        INSERT INTO activity_logs (
            workspace_id,
            settings_account_id,
            smart_account_id,
            activity_type,
            title,
            tx_sig,
            metadata
        )
        VALUES ($1, $2, $3, 'SMART_ACCOUNT_FUNDED',
                'Smart account funded', $4, $5)
        ON CONFLICT (activity_type, tx_sig)
            WHERE tx_sig IS NOT NULL
        DO NOTHING
        "#,
    )
    .bind(workspace_id)
    .bind(refreshed.settings_account_id)
    .bind(refreshed.id)
    .bind(signature.to_string())
    .bind(serde_json::json!({
        "walletAddress": wallet_address.to_string(),
        "smartAccountPda": smart_account_pda.to_string(),
        "amountLamports": amount_lamports.to_string()
    }))
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(FundSmartAccountSubmittedResponse {
        smart_account: refreshed.into(),
        tx_sig: signature.to_string(),
    }))
}

pub async fn refresh_smart_account(
    State(state): State<AppState>,
    Path(id_or_pda): Path<String>,
) -> Result<Json<SmartAccountEnvelope>, ApiError> {
    let smart_account = sqlx::query_as::<_, SmartAccountRow>(
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
        WHERE id::text = $1 OR pda = $1
        "#,
    )
    .bind(id_or_pda.trim())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Smart account not found"))?;

    let refreshed = refresh_smart_account_balance(&state, &smart_account).await?;

    Ok(Json(SmartAccountEnvelope {
        smart_account: refreshed.into(),
    }))
}
