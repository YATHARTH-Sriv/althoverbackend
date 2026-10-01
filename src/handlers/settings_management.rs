use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Signature};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        PROGRAM_ID, build_unsigned_transaction_base64, derive_settings_pda,
        verify_confirmed_transaction,
    },
    squadsaccounts::{
        SettingsAccount, build_add_signer_instruction, build_change_threshold_instruction,
        build_remove_signer_instruction, decode_settings_account,
    },
    validation::parse_pubkey,
};

const VOTE_PERMISSION: u8 = 2;
const ALL_PERMISSIONS: u8 = 7;
const INVITE_TTL_DAYS: i64 = 14;

#[derive(Debug, FromRow)]
struct IndexedSettings {
    id: Uuid,
    workspace_id: Uuid,
    pda: String,
}

#[derive(Debug, FromRow)]
struct SettingsRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    pda: String,
    seed: String,
    settings_authority: String,
    threshold: i32,
    time_lock: i64,
    transaction_index: String,
    stale_transaction_index: String,
    creation_tx_sig: Option<String>,
    created_by_wallet: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSignerResponse {
    id: Uuid,
    wallet_address: String,
    role: String,
    permissions_mask: i32,
    label: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    id: Uuid,
    name: String,
    #[serde(rename = "type")]
    workspace_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    pda: String,
    seed: String,
    settings_authority: String,
    threshold: i32,
    time_lock: i64,
    transaction_index: String,
    stale_transaction_index: String,
    creation_tx_sig: Option<String>,
    created_by_wallet: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    workspace: WorkspaceResponse,
    signers: Vec<SettingsSignerResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeThresholdBuildRequest {
    wallet_address: String,
    settings_pda: String,
    new_threshold: u16,
    memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeThresholdSubmittedRequest {
    wallet_address: String,
    settings_pda: String,
    new_threshold: u16,
    tx_sig: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSignerBuildRequest {
    wallet_address: String,
    settings_pda: String,
    signer: String,
    permissions_mask: Option<u8>,
    memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSignerSubmittedRequest {
    wallet_address: String,
    settings_pda: String,
    signer: String,
    permissions_mask: Option<u8>,
    name: Option<String>,
    email: Option<String>,
    designation: Option<String>,
    tx_sig: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSignerBuildRequest {
    wallet_address: String,
    settings_pda: String,
    signer: String,
    memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSignerSubmittedRequest {
    wallet_address: String,
    settings_pda: String,
    signer: String,
    tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSettingsTransactionResponse {
    wallet_address: String,
    settings_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions_mask: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_threshold: Option<u16>,
    memo: String,
    transaction_base64: String,
}

#[derive(Debug, Serialize)]
pub struct SubmittedSettingsResponse {
    settings: SettingsResponse,
}

#[derive(Debug, FromRow)]
struct InviteRow {
    token: String,
    status: String,
    wallet_address: String,
    email: Option<String>,
    name: Option<String>,
    designation: Option<String>,
    permissions_mask: i32,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteResponse {
    token: String,
    invite_url: String,
    status: String,
    wallet_address: String,
    email: Option<String>,
    name: Option<String>,
    designation: Option<String>,
    permissions_mask: i32,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AddSignerSubmittedResponse {
    settings: SettingsResponse,
    invite: InviteResponse,
}

fn parse_signature(value: &str) -> Result<Signature, ApiError> {
    Signature::from_str(value.trim()).map_err(|_| bad_request("txSig is invalid"))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn validate_permissions(mask: u8) -> Result<(), ApiError> {
    if mask == 0 || mask & !ALL_PERMISSIONS != 0 {
        return Err(bad_request(
            "permissionsMask must use Initiate (1), Approve (2), Execute (4), or a valid combination",
        ));
    }

    Ok(())
}

async fn load_indexed_settings(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<IndexedSettings, ApiError> {
    sqlx::query_as::<_, IndexedSettings>(
        r#"
        SELECT id, workspace_id, pda
        FROM settings_accounts
        WHERE pda = $1
        "#,
    )
    .bind(settings_pda.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account is not indexed yet"))
}

async fn fetch_chain_settings(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<SettingsAccount, ApiError> {
    let account = state
        .rpc
        .get_account(settings_pda)
        .await
        .map_err(internal_error)?;

    let settings =
        decode_settings_account(&account.owner, &account.data).map_err(internal_error)?;

    if derive_settings_pda(settings.seed).0 != *settings_pda {
        return Err(bad_request(
            "Settings account does not match its on-chain seed",
        ));
    }

    Ok(settings)
}

fn ensure_authority(
    chain_settings: &SettingsAccount,
    wallet_address: &Pubkey,
) -> Result<(), ApiError> {
    if chain_settings.settings_authority != *wallet_address {
        return Err(bad_request(
            "Only the settings authority can perform this action directly",
        ));
    }

    Ok(())
}

async fn build_unsigned_settings_transaction(
    state: &AppState,
    authority: Pubkey,
    instruction: Instruction,
) -> Result<String, ApiError> {
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;

    build_unsigned_transaction_base64(authority, blockhash, instruction).map_err(internal_error)
}

async fn sync_settings_from_chain(
    state: &AppState,
    indexed: &IndexedSettings,
    chain: &SettingsAccount,
) -> Result<(), ApiError> {
    let mut transaction = state.db.begin().await.map_err(internal_error)?;

    sqlx::query(
        r#"
        UPDATE settings_accounts
        SET
            seed = $2::numeric,
            settings_authority = $3,
            threshold = $4,
            time_lock = $5,
            transaction_index = $6::numeric,
            stale_transaction_index = $7::numeric,
            updated_at = NOW()
        WHERE id = $1
        "#,
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
        let fallback_label = if role == "OWNER" {
            "Authority".to_owned()
        } else {
            format!("Signer {}", index + 1)
        };

        signer_addresses.push(address.clone());

        sqlx::query(
            r#"
            INSERT INTO settings_signers (
                settings_account_id,
                wallet_address,
                role,
                permissions_mask,
                label
            )
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (settings_account_id, wallet_address)
            DO UPDATE SET
                role = EXCLUDED.role,
                permissions_mask = EXCLUDED.permissions_mask,
                label = COALESCE(settings_signers.label, EXCLUDED.label),
                updated_at = NOW()
            "#,
        )
        .bind(indexed.id)
        .bind(address)
        .bind(role)
        .bind(i32::from(signer.permissions.mask))
        .bind(fallback_label)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }

    sqlx::query(
        r#"
        DELETE FROM settings_signers
        WHERE settings_account_id = $1
          AND NOT (wallet_address = ANY($2))
        "#,
    )
    .bind(indexed.id)
    .bind(&signer_addresses)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    transaction.commit().await.map_err(internal_error)?;
    Ok(())
}

pub(crate) async fn refresh_settings_by_pda(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<(), ApiError> {
    let indexed = load_indexed_settings(state, settings_pda).await?;
    let chain = fetch_chain_settings(state, settings_pda).await?;
    sync_settings_from_chain(state, &indexed, &chain).await
}

async fn fetch_settings_response(
    state: &AppState,
    settings_id: Uuid,
) -> Result<SettingsResponse, ApiError> {
    let settings = sqlx::query_as::<_, SettingsRow>(
        r#"
        SELECT
            id,
            workspace_id,
            name,
            pda,
            seed::text AS seed,
            settings_authority,
            threshold,
            time_lock,
            transaction_index::text AS transaction_index,
            stale_transaction_index::text AS stale_transaction_index,
            creation_tx_sig,
            created_by_wallet,
            created_at,
            updated_at
        FROM settings_accounts
        WHERE id = $1
        "#,
    )
    .bind(settings_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;

    let workspace = sqlx::query_as::<_, WorkspaceResponse>(
        r#"
        SELECT id, name, workspace_type
        FROM workspaces
        WHERE id = $1
        "#,
    )
    .bind(settings.workspace_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;

    let signers = sqlx::query_as::<_, SettingsSignerResponse>(
        r#"
        SELECT id, wallet_address, role, permissions_mask, label
        FROM settings_signers
        WHERE settings_account_id = $1
        ORDER BY created_at ASC
        "#,
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

async fn verify_submitted_transaction(
    state: &AppState,
    tx_sig: &str,
    authority: &Pubkey,
    settings_pda: &Pubkey,
) -> Result<Signature, ApiError> {
    let signature = parse_signature(tx_sig)?;

    verify_confirmed_transaction(
        state.rpc.as_ref(),
        &signature,
        authority,
        &[*settings_pda, PROGRAM_ID],
    )
    .await
    .map_err(bad_request)?;

    Ok(signature)
}

async fn log_activity(
    state: &AppState,
    indexed: &IndexedSettings,
    activity_type: &str,
    title: &str,
    tx_sig: &Signature,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO activity_logs (
            workspace_id,
            settings_account_id,
            activity_type,
            title,
            tx_sig,
            metadata
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (activity_type, tx_sig)
            WHERE tx_sig IS NOT NULL
        DO NOTHING
        "#,
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

pub async fn build_change_threshold(
    State(state): State<AppState>,
    Json(payload): Json<ChangeThresholdBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;

    load_indexed_settings(&state, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;
    ensure_authority(&chain, &authority)?;

    let vote_signer_count = chain
        .signers
        .iter()
        .filter(|signer| signer.permissions.mask & VOTE_PERMISSION != 0)
        .count();

    if payload.new_threshold == 0 || usize::from(payload.new_threshold) > vote_signer_count {
        return Err(bad_request(format!(
            "Threshold must be between 1 and {vote_signer_count} approve-capable signer{}",
            if vote_signer_count == 1 { "" } else { "s" }
        )));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| "Hover Agent threshold update".to_owned());
    let instruction = build_change_threshold_instruction(
        settings_pda,
        authority,
        payload.new_threshold,
        Some(memo.clone()),
    )
    .map_err(internal_error)?;
    let transaction_base64 =
        build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: None,
        permissions_mask: None,
        new_threshold: Some(payload.new_threshold),
        memo,
        transaction_base64,
    }))
}

pub async fn change_threshold_submitted(
    State(state): State<AppState>,
    Json(payload): Json<ChangeThresholdSubmittedRequest>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let indexed = load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;

    ensure_authority(&chain, &authority)?;
    if chain.threshold != payload.new_threshold {
        return Err(bad_request(
            "Confirmed transaction did not apply the requested threshold",
        ));
    }

    sync_settings_from_chain(&state, &indexed, &chain).await?;
    log_activity(
        &state,
        &indexed,
        "THRESHOLD_CHANGED",
        "Threshold changed",
        &signature,
        serde_json::json!({ "newThreshold": payload.new_threshold }),
    )
    .await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: fetch_settings_response(&state, indexed.id).await?,
    }))
}

pub async fn build_add_signer(
    State(state): State<AppState>,
    Json(payload): Json<AddSignerBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let permissions_mask = payload.permissions_mask.unwrap_or(ALL_PERMISSIONS);

    validate_permissions(permissions_mask)?;
    load_indexed_settings(&state, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;
    ensure_authority(&chain, &authority)?;

    if chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "This wallet is already a signer on the settings account",
        ));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| "Hover Agent signer update addition".to_owned());
    let instruction = build_add_signer_instruction(
        settings_pda,
        authority,
        signer,
        permissions_mask,
        Some(memo.clone()),
    )
    .map_err(internal_error)?;
    let transaction_base64 =
        build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: Some(signer.to_string()),
        permissions_mask: Some(permissions_mask),
        new_threshold: None,
        memo,
        transaction_base64,
    }))
}

struct SignerInviteInput<'a> {
    indexed: &'a IndexedSettings,
    authority: &'a Pubkey,
    signer: &'a Pubkey,
    permissions_mask: u8,
    name: Option<String>,
    email: Option<String>,
    designation: Option<String>,
}

async fn upsert_signer_profile_and_invite(
    state: &AppState,
    input: SignerInviteInput<'_>,
) -> Result<InviteResponse, ApiError> {
    let signer_address = input.signer.to_string();
    let label = input
        .name
        .clone()
        .or_else(|| input.designation.clone())
        .unwrap_or_else(|| "Signer".to_owned());
    let mut transaction = state.db.begin().await.map_err(internal_error)?;

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{}:{signer_address}", input.indexed.id))
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;

    let existing_user_id =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT user_id FROM wallets WHERE address = $1")
            .bind(&signer_address)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(internal_error)?
            .flatten();

    let user_id = if let Some(user_id) = existing_user_id {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            UPDATE users
            SET
                name = COALESCE($2, name),
                email = COALESCE($3, email),
                designation = COALESCE($4, designation),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(input.name.as_deref())
        .bind(input.email.as_deref())
        .bind(input.designation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO users (name, email, designation)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(input.name.as_deref())
        .bind(input.email.as_deref())
        .bind(input.designation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    };

    sqlx::query(
        r#"
        INSERT INTO wallets (address, user_id, label)
        VALUES ($1, $2, $3)
        ON CONFLICT (address)
        DO UPDATE SET
            user_id = EXCLUDED.user_id,
            label = EXCLUDED.label,
            updated_at = NOW()
        "#,
    )
    .bind(&signer_address)
    .bind(user_id)
    .bind(&label)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    sqlx::query(
        r#"
        INSERT INTO workspace_members (workspace_id, user_id, role)
        VALUES ($1, $2, 'MEMBER')
        ON CONFLICT (workspace_id, user_id)
        DO UPDATE SET role = 'MEMBER', updated_at = NOW()
        "#,
    )
    .bind(input.indexed.workspace_id)
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    sqlx::query(
        r#"
        UPDATE settings_signers
        SET
            label = $3,
            role = 'EXTERNAL',
            permissions_mask = $4,
            updated_at = NOW()
        WHERE settings_account_id = $1 AND wallet_address = $2
        "#,
    )
    .bind(input.indexed.id)
    .bind(&signer_address)
    .bind(&label)
    .bind(i32::from(input.permissions_mask))
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    let existing_invite_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id
        FROM workspace_invites
        WHERE settings_account_id = $1
          AND wallet_address = $2
          AND status = 'PENDING'
        FOR UPDATE
        "#,
    )
    .bind(input.indexed.id)
    .bind(&signer_address)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?;

    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);
    let invite = if let Some(invite_id) = existing_invite_id {
        sqlx::query_as::<_, InviteRow>(
            r#"
            UPDATE workspace_invites
            SET
                email = $2,
                name = $3,
                designation = $4,
                permissions_mask = $5,
                invited_by_wallet = $6,
                expires_at = $7,
                updated_at = NOW()
            WHERE id = $1
            RETURNING token, status, wallet_address, email, name, designation,
                      permissions_mask, expires_at
            "#,
        )
        .bind(invite_id)
        .bind(input.email.as_deref())
        .bind(input.name.as_deref())
        .bind(input.designation.as_deref())
        .bind(i32::from(input.permissions_mask))
        .bind(input.authority.to_string())
        .bind(expires_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    } else {
        let mut token_bytes = [0_u8; 24];
        getrandom::fill(&mut token_bytes).map_err(internal_error)?;
        let token = hex::encode(token_bytes);

        sqlx::query_as::<_, InviteRow>(
            r#"
            INSERT INTO workspace_invites (
                workspace_id,
                settings_account_id,
                token,
                wallet_address,
                email,
                name,
                designation,
                permissions_mask,
                invited_by_wallet,
                expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING token, status, wallet_address, email, name, designation,
                      permissions_mask, expires_at
            "#,
        )
        .bind(input.indexed.workspace_id)
        .bind(input.indexed.id)
        .bind(token)
        .bind(&signer_address)
        .bind(input.email.as_deref())
        .bind(input.name.as_deref())
        .bind(input.designation.as_deref())
        .bind(i32::from(input.permissions_mask))
        .bind(input.authority.to_string())
        .bind(expires_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    };

    transaction.commit().await.map_err(internal_error)?;

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

pub async fn add_signer_submitted(
    State(state): State<AppState>,
    Json(payload): Json<AddSignerSubmittedRequest>,
) -> Result<Json<AddSignerSubmittedResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let permissions_mask = payload.permissions_mask.unwrap_or(ALL_PERMISSIONS);

    validate_permissions(permissions_mask)?;
    let indexed = load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;
    ensure_authority(&chain, &authority)?;

    let confirmed_signer = chain
        .signers
        .iter()
        .find(|existing| existing.key == signer)
        .ok_or_else(|| bad_request("Confirmed transaction did not add the requested signer"))?;

    if confirmed_signer.permissions.mask != permissions_mask {
        return Err(bad_request(
            "Confirmed transaction did not apply the requested signer permissions",
        ));
    }

    sync_settings_from_chain(&state, &indexed, &chain).await?;

    let name = normalize_optional_string(payload.name);
    let email = normalize_optional_string(payload.email);
    let designation = normalize_optional_string(payload.designation);
    let invite = upsert_signer_profile_and_invite(
        &state,
        SignerInviteInput {
            indexed: &indexed,
            authority: &authority,
            signer: &signer,
            permissions_mask,
            name: name.clone(),
            email: email.clone(),
            designation: designation.clone(),
        },
    )
    .await?;

    log_activity(
        &state,
        &indexed,
        "SIGNER_ADDED",
        "Signer added",
        &signature,
        serde_json::json!({
            "signer": signer.to_string(),
            "permissionsMask": permissions_mask,
            "name": name,
            "email": email,
            "designation": designation
        }),
    )
    .await?;

    Ok(Json(AddSignerSubmittedResponse {
        settings: fetch_settings_response(&state, indexed.id).await?,
        invite,
    }))
}

pub async fn build_remove_signer(
    State(state): State<AppState>,
    Json(payload): Json<RemoveSignerBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;

    load_indexed_settings(&state, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;
    ensure_authority(&chain, &authority)?;

    if !chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "This wallet is not currently a signer on the settings account",
        ));
    }
    if chain.signers.len() <= 1 {
        return Err(bad_request("Cannot remove the last signer"));
    }

    let remaining_vote_signers = chain
        .signers
        .iter()
        .filter(|existing| {
            existing.key != signer && existing.permissions.mask & VOTE_PERMISSION != 0
        })
        .count();

    if usize::from(chain.threshold) > remaining_vote_signers {
        return Err(bad_request(format!(
            "Lower the threshold before removing this signer. Current threshold is {}, but only {remaining_vote_signers} approve-capable signer{} would remain.",
            chain.threshold,
            if remaining_vote_signers == 1 { "" } else { "s" }
        )));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| format!("Hover Agent signer update removing {signer}"));
    let instruction =
        build_remove_signer_instruction(settings_pda, authority, signer, Some(memo.clone()))
            .map_err(internal_error)?;
    let transaction_base64 =
        build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: Some(signer.to_string()),
        permissions_mask: None,
        new_threshold: None,
        memo,
        transaction_base64,
    }))
}

pub async fn remove_signer_submitted(
    State(state): State<AppState>,
    Json(payload): Json<RemoveSignerSubmittedRequest>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let indexed = load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda).await?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;

    ensure_authority(&chain, &authority)?;
    if chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "Confirmed transaction did not remove the requested signer",
        ));
    }

    sync_settings_from_chain(&state, &indexed, &chain).await?;

    sqlx::query(
        r#"
        UPDATE workspace_invites
        SET status = 'REVOKED', updated_at = NOW()
        WHERE settings_account_id = $1
          AND wallet_address = $2
          AND status = 'PENDING'
        "#,
    )
    .bind(indexed.id)
    .bind(signer.to_string())
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    log_activity(
        &state,
        &indexed,
        "SIGNER_REMOVED",
        "Signer removed",
        &signature,
        serde_json::json!({ "signer": signer.to_string() }),
    )
    .await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: fetch_settings_response(&state, indexed.id).await?,
    }))
}

pub async fn refresh_settings(
    State(state): State<AppState>,
    Path(id_or_pda): Path<String>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let indexed = sqlx::query_as::<_, IndexedSettings>(
        r#"
        SELECT id, workspace_id, pda
        FROM settings_accounts
        WHERE id::text = $1 OR pda = $1
        "#,
    )
    .bind(id_or_pda.trim())
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Settings account not found"))?;

    let settings_pda = parse_pubkey(&indexed.pda, "settingsPda")?;
    let chain = fetch_chain_settings(&state, &settings_pda).await?;
    sync_settings_from_chain(&state, &indexed, &chain).await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: fetch_settings_response(&state, indexed.id).await?,
    }))
}
