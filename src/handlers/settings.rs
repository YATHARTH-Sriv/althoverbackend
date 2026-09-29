use std::{collections::HashSet, str::FromStr};

use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use sqlx::prelude::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        PROGRAM_ID, build_unsigned_transaction_base64, derive_settings_pda,
        verify_confirmed_transaction,
    },
    squadsaccounts::{
        CreateSmartAccountArgs, Permissions, SmartAccountSigner,
        build_create_smart_account_instruction, decode_settings_account, fetch_program_config,
    },
};

const INITIATE_PERMISSION: u8 = 1;
const VOTE_PERMISSION: u8 = 2;
const EXECUTE_PERMISSION: u8 = 4;
const ALL_PERMISSIONS: u8 = 7;
const MAX_TIME_LOCK: u32 = 3 * 30 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateSettingsRequest {
    pub wallet_address: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub threshold: Option<u16>,
    pub time_lock: Option<u32>,
    pub signers: Option<Vec<SettingsSignerInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SettingsSignerInput {
    Address(String),
    Detailed(SettingsSignerObject),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSignerObject {
    pub wallet_address: String,
    pub permissions_mask: Option<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSignerResponse {
    pub wallet_address: String,
    pub permissions_mask: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateSettingsResponse {
    pub wallet_address: String,
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    pub threshold: u16,
    pub time_lock: u32,
    pub settings_pda: String,
    pub seed: String,
    pub signers: Vec<SettingsSignerResponse>,
    pub transaction_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSubmittedRequest {
    pub wallet_address: String,
    pub settings_pda: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub tx_sig: String,
}

#[derive(Debug, FromRow)]
struct StoredSettingsRow {
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
pub struct StoredSettingsSigner {
    id: Uuid,
    wallet_address: String,
    role: String,
    permissions_mask: i32,
    label: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSettingsResponse {
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
    signers: Vec<StoredSettingsSigner>,
}

#[derive(Debug, Serialize)]
pub struct SettingsSubmittedResponse {
    settings: StoredSettingsResponse,
}

pub async fn build_create_settings(
    State(state): State<AppState>,
    Json(payload): Json<BuildCreateSettingsRequest>,
) -> Result<Json<BuildCreateSettingsResponse>, ApiError> {
    let creator = parse_pubkey(&payload.wallet_address, "walletAddress")?;

    let name = normalize_optional_string(payload.name).unwrap_or_else(|| "My Hover".to_owned());

    let email = normalize_optional_string(payload.email);
    let threshold = payload.threshold.unwrap_or(1);
    let time_lock = payload.time_lock.unwrap_or(0);

    if time_lock > MAX_TIME_LOCK {
        return Err(bad_request(format!(
            "timeLock cannot exceed {MAX_TIME_LOCK} seconds"
        )));
    }

    let mut signers = Vec::new();
    let mut signer_keys = HashSet::new();

    for input in payload.signers.unwrap_or_default() {
        let (address, permissions_mask) = match input {
            SettingsSignerInput::Address(address) => (address, ALL_PERMISSIONS),
            SettingsSignerInput::Detailed(signer) => (
                signer.wallet_address,
                signer.permissions_mask.unwrap_or(ALL_PERMISSIONS),
            ),
        };

        let signer_key = parse_pubkey(&address, "signer.walletAddress")?;

        validate_permission_mask(permissions_mask)?;

        if !signer_keys.insert(signer_key) {
            return Err(bad_request("Duplicate signer wallet address"));
        }

        signers.push(SmartAccountSigner {
            key: signer_key,
            permissions: Permissions {
                mask: permissions_mask,
            },
        });
    }

    if !signer_keys.contains(&creator) {
        signers.insert(
            0,
            SmartAccountSigner {
                key: creator,
                permissions: Permissions {
                    mask: ALL_PERMISSIONS,
                },
            },
        );
    }

    validate_signer_permissions(&signers)?;

    let vote_signer_count = signers
        .iter()
        .filter(|signer| signer.permissions.mask & VOTE_PERMISSION != 0)
        .count();

    if threshold == 0 || usize::from(threshold) > vote_signer_count {
        return Err(bad_request(format!(
            "Threshold must be between 1 and {vote_signer_count} \
             approve-capable signer{}",
            if vote_signer_count == 1 { "" } else { "s" }
        )));
    }

    let (program_config_pda, program_config) = fetch_program_config(state.rpc.as_ref())
        .await
        .map_err(internal_error)?;

    let seed = program_config
        .smart_account_index
        .checked_add(1)
        .ok_or_else(|| internal_error("Smart account index overflow"))?;

    let (settings_pda, _) = derive_settings_pda(seed);

    let instruction = build_create_smart_account_instruction(
        program_config_pda,
        program_config.treasury,
        creator,
        settings_pda,
        CreateSmartAccountArgs {
            settings_authority: Some(creator),
            threshold,
            signers: signers.clone(),
            time_lock,
            rent_collector: Some(creator),
            memo: Some(name.clone()),
        },
    )
    .map_err(internal_error)?;

    let recent_blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;

    let transaction_base64 =
        build_unsigned_transaction_base64(creator, recent_blockhash, instruction)
            .map_err(internal_error)?;

    let signer_response = signers
        .into_iter()
        .map(|signer| SettingsSignerResponse {
            wallet_address: signer.key.to_string(),
            permissions_mask: signer.permissions.mask,
        })
        .collect();

    Ok(Json(BuildCreateSettingsResponse {
        wallet_address: creator.to_string(),
        name,
        email,
        threshold,
        time_lock,
        settings_pda: settings_pda.to_string(),
        seed: seed.to_string(),
        signers: signer_response,
        transaction_base64,
    }))
}

pub async fn settings_submitted(
    State(state): State<AppState>,
    Json(payload): Json<SettingsSubmittedRequest>,
) -> Result<Json<SettingsSubmittedResponse>, ApiError> {
    let wallet_address = parse_pubkey(&payload.wallet_address, "walletAddress")?;

    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;

    let tx_signature =
        Signature::from_str(payload.tx_sig.trim()).map_err(|_| bad_request("txSig is invalid"))?;

    let name = normalize_optional_string(payload.name).unwrap_or_else(|| "My Hover".to_owned());

    let email = normalize_optional_string(payload.email);

    verify_confirmed_transaction(
        state.rpc.as_ref(),
        &tx_signature,
        &wallet_address,
        &[settings_pda, PROGRAM_ID],
    )
    .await
    .map_err(bad_request)?;

    let chain_account = state
        .rpc
        .get_account(&settings_pda)
        .await
        .map_err(internal_error)?;

    let chain_settings = decode_settings_account(&chain_account.owner, &chain_account.data)
        .map_err(internal_error)?;

    if chain_settings.settings_authority != wallet_address {
        return Err(bad_request(
            "Submitted settings account does not belong to this wallet",
        ));
    }

    let expected_settings_pda = derive_settings_pda(chain_settings.seed).0;

    if expected_settings_pda != settings_pda {
        return Err(bad_request(
            "Settings account does not match its on-chain seed",
        ));
    }

    let mut database_transaction = state.db.begin().await.map_err(internal_error)?;

    // Prevent two identical submitted requests from creating two
    // workspaces concurrently.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(settings_pda.to_string())
        .execute(&mut *database_transaction)
        .await
        .map_err(internal_error)?;

    let existing_workspace_id = sqlx::query_scalar::<_, Uuid>(
        r#"
            SELECT workspace_id
            FROM settings_accounts
            WHERE pda = $1
            "#,
    )
    .bind(settings_pda.to_string())
    .fetch_optional(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    let workspace_id = if let Some(workspace_id) = existing_workspace_id {
        workspace_id
    } else {
        let workspace_id = sqlx::query_scalar::<_, Uuid>(
            r#"
                INSERT INTO workspaces (
                    name,
                    email,
                    workspace_type,
                    created_by_wallet
                )
                VALUES ($1, $2, 'BUSINESS', $3)
                RETURNING id
                "#,
        )
        .bind(&name)
        .bind(email.as_deref())
        .bind(wallet_address.to_string())
        .fetch_one(&mut *database_transaction)
        .await
        .map_err(internal_error)?;

        let user_id = sqlx::query_scalar::<_, Option<Uuid>>(
            r#"
                    SELECT user_id
                    FROM wallets
                    WHERE address = $1
                    "#,
        )
        .bind(wallet_address.to_string())
        .fetch_optional(&mut *database_transaction)
        .await
        .map_err(internal_error)?
        .flatten();

        if let Some(user_id) = user_id {
            sqlx::query(
                r#"
                    INSERT INTO workspace_members (
                        workspace_id,
                        user_id,
                        role
                    )
                    VALUES ($1, $2, 'OWNER')
                    ON CONFLICT (workspace_id, user_id)
                    DO UPDATE SET
                        role = 'OWNER',
                        updated_at = NOW()
                    "#,
            )
            .bind(workspace_id)
            .bind(user_id)
            .execute(&mut *database_transaction)
            .await
            .map_err(internal_error)?;
        }

        workspace_id
    };

    let settings_record = sqlx::query_as::<_, StoredSettingsRow>(
        r#"
            INSERT INTO settings_accounts (
                workspace_id,
                name,
                pda,
                seed,
                settings_authority,
                threshold,
                time_lock,
                transaction_index,
                stale_transaction_index,
                creation_tx_sig,
                created_by_wallet
            )
            VALUES (
                $1,
                $2,
                $3,
                $4::numeric,
                $5,
                $6,
                $7,
                $8::numeric,
                $9::numeric,
                $10,
                $11
            )
            ON CONFLICT (pda)
            DO UPDATE SET
                name = EXCLUDED.name,
                seed = EXCLUDED.seed,
                settings_authority =
                    EXCLUDED.settings_authority,
                threshold = EXCLUDED.threshold,
                time_lock = EXCLUDED.time_lock,
                transaction_index =
                    EXCLUDED.transaction_index,
                stale_transaction_index =
                    EXCLUDED.stale_transaction_index,
                creation_tx_sig =
                    COALESCE(
                        settings_accounts.creation_tx_sig,
                        EXCLUDED.creation_tx_sig
                    ),
                updated_at = NOW()
            RETURNING
                id,
                workspace_id,
                name,
                pda,
                seed::text AS seed,
                settings_authority,
                threshold,
                time_lock,
                transaction_index::text
                    AS transaction_index,
                stale_transaction_index::text
                    AS stale_transaction_index,
                creation_tx_sig,
                created_by_wallet,
                created_at,
                updated_at
            "#,
    )
    .bind(workspace_id)
    .bind(&name)
    .bind(settings_pda.to_string())
    .bind(chain_settings.seed.to_string())
    .bind(chain_settings.settings_authority.to_string())
    .bind(i32::from(chain_settings.threshold))
    .bind(i64::from(chain_settings.time_lock))
    .bind(chain_settings.transaction_index.to_string())
    .bind(chain_settings.stale_transaction_index.to_string())
    .bind(tx_signature.to_string())
    .bind(wallet_address.to_string())
    .fetch_one(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    let mut signer_addresses = Vec::with_capacity(chain_settings.signers.len());

    for (index, signer) in chain_settings.signers.iter().enumerate() {
        let signer_address = signer.key.to_string();
        signer_addresses.push(signer_address.clone());

        let is_owner = signer.key == chain_settings.settings_authority;

        let role = if is_owner { "OWNER" } else { "EXTERNAL" };

        let label = if is_owner {
            "Authority".to_owned()
        } else {
            format!("Signer {}", index + 1)
        };

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
            ON CONFLICT (
                settings_account_id,
                wallet_address
            )
            DO UPDATE SET
                role = EXCLUDED.role,
                permissions_mask =
                    EXCLUDED.permissions_mask,
                label = EXCLUDED.label,
                updated_at = NOW()
            "#,
        )
        .bind(settings_record.id)
        .bind(signer_address)
        .bind(role)
        .bind(i32::from(signer.permissions.mask))
        .bind(label)
        .execute(&mut *database_transaction)
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
    .bind(settings_record.id)
    .bind(&signer_addresses)
    .execute(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

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
        VALUES (
            $1,
            $2,
            'SETTINGS_CREATED',
            'Settings account created',
            $3,
            $4
        )
        ON CONFLICT (activity_type, tx_sig)
            WHERE tx_sig IS NOT NULL
        DO NOTHING
        "#,
    )
    .bind(workspace_id)
    .bind(settings_record.id)
    .bind(tx_signature.to_string())
    .bind(serde_json::json!({
        "settingsPda": settings_pda.to_string()
    }))
    .execute(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    let signers = sqlx::query_as::<_, StoredSettingsSigner>(
        r#"
            SELECT
                id,
                wallet_address,
                role,
                permissions_mask,
                label
            FROM settings_signers
            WHERE settings_account_id = $1
            ORDER BY created_at ASC
            "#,
    )
    .bind(settings_record.id)
    .fetch_all(&mut *database_transaction)
    .await
    .map_err(internal_error)?;

    database_transaction
        .commit()
        .await
        .map_err(internal_error)?;

    Ok(Json(SettingsSubmittedResponse {
        settings: StoredSettingsResponse {
            id: settings_record.id,
            workspace_id: settings_record.workspace_id,
            name: settings_record.name,
            pda: settings_record.pda,
            seed: settings_record.seed,
            settings_authority: settings_record.settings_authority,
            threshold: settings_record.threshold,
            time_lock: settings_record.time_lock,
            transaction_index: settings_record.transaction_index,
            stale_transaction_index: settings_record.stale_transaction_index,
            creation_tx_sig: settings_record.creation_tx_sig,
            created_by_wallet: settings_record.created_by_wallet,
            created_at: settings_record.created_at,
            updated_at: settings_record.updated_at,
            signers,
        },
    }))
}

/// Helpers

fn parse_pubkey(value: &str, field: &str) -> Result<Pubkey, ApiError> {
    let value = value.trim();

    if value.is_empty() {
        return Err(bad_request(format!("{field} is required")));
    }

    Pubkey::from_str(value).map_err(|_| bad_request(format!("{field} is invalid")))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn validate_permission_mask(mask: u8) -> Result<(), ApiError> {
    if mask == 0 || mask & !ALL_PERMISSIONS != 0 {
        return Err(bad_request(
            "signer.permissionsMask must use Initiate (1), \
             Approve (2), Execute (4), or a valid combination",
        ));
    }

    Ok(())
}

fn validate_signer_permissions(signers: &[SmartAccountSigner]) -> Result<(), ApiError> {
    let has_initiator = signers
        .iter()
        .any(|signer| signer.permissions.mask & INITIATE_PERMISSION != 0);

    let has_voter = signers
        .iter()
        .any(|signer| signer.permissions.mask & VOTE_PERMISSION != 0);

    let has_executor = signers
        .iter()
        .any(|signer| signer.permissions.mask & EXECUTE_PERMISSION != 0);

    if !has_initiator {
        return Err(bad_request(
            "At least one signer must be able to initiate transactions",
        ));
    }

    if !has_voter {
        return Err(bad_request(
            "At least one signer must be able to approve transactions",
        ));
    }

    if !has_executor {
        return Err(bad_request(
            "At least one signer must be able to execute transactions",
        ));
    }

    Ok(())
}
