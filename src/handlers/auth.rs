use std::str::FromStr;

use axum::{Json, extract::State};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthChallengeRequest {
    wallet_address: String,
    // purpose: Option<String>,
    // settings_pda: Option<String>,
    // code_challenge: Option<String>,
    // extension_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthChallengeResponse {
    wallet_address: String,
    message: String,
    nonce_expires_at: DateTime<Utc>,
}

fn normalize_wallet_address(value: &str) -> Result<String, ApiError> {
    Pubkey::from_str(value.trim())
        .map(|pubkey| pubkey.to_string())
        .map_err(|_| bad_request("walletAddress is invalid"))
}

fn generate_nonce() -> Result<String, ApiError> {
    let mut bytes = [0u8; 16];

    getrandom::fill(&mut bytes).map_err(internal_error)?;

    Ok(hex::encode(bytes))
}

pub async fn auth_challenge(
    State(state): State<AppState>,
    Json(payload): Json<AuthChallengeRequest>,
) -> Result<Json<AuthChallengeResponse>, ApiError> {
    // println!("Wallet Address : {:?}", payload.wallet_address)
    let walletaddress = normalize_wallet_address(&payload.wallet_address)?;
    let nonce = generate_nonce()?;
    let message =
        format!("Hover Agent wallet login\n\n  Wallet: {walletaddress}\n  Nonce: {nonce}");
    let nonce_expires_at = Utc::now() + Duration::minutes(10);
    sqlx::query(
        r#"
        INSERT INTO wallets (
            address,
            nonce,
            nonce_expires_at
        )
        VALUES ($1, $2, $3)
        ON CONFLICT (address)
        DO UPDATE SET
            nonce = EXCLUDED.nonce,
            nonce_expires_at = EXCLUDED.nonce_expires_at,
            updated_at = NOW()
        "#,
    )
    .bind(&walletaddress)
    .bind(&message)
    .bind(nonce_expires_at)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(AuthChallengeResponse {
        wallet_address: walletaddress,
        message,
        nonce_expires_at,
    }))
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    id: Uuid,
    name: Option<String>,
    email: Option<String>,
    designation: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct WalletForVerification {
    id: Uuid,
    user_id: Option<Uuid>,
    nonce: Option<String>,
    nonce_expires_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthVerifyRequest {
    wallet_address: String,
    message: String,
    signature: String,
    name: Option<String>,
    email: Option<String>,
    designation: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wallet {
    id: Uuid,
    address: String,
    label: Option<String>,
    user_id: Option<Uuid>,
    verified_at: Option<DateTime<Utc>>,
    nonce: Option<String>,
    nonce_expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthVerifyResponse {
    user: User,
    wallet: Wallet,
}

fn verify_wallet_signature(
    wallet_address: &str,
    message: &str,
    signature_input: &str,
) -> Result<(), ApiError> {
    let public_key =
        Pubkey::from_str(wallet_address).map_err(|_| bad_request("walletAddress is invalid"))?;

    let signature_bytes = STANDARD
        .decode(signature_input.trim())
        .map_err(|_| bad_request("Invalid wallet signature"))?;

    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| bad_request("Invalid wallet signature"))?;

    let verifying_key = VerifyingKey::from_bytes(&public_key.to_bytes())
        .map_err(|_| bad_request("Invalid wallet signature"))?;

    verifying_key
        .verify_strict(message.as_bytes(), &signature)
        .map_err(|_| bad_request("Invalid wallet signature"))
}

pub async fn auth_verify(
    State(state): State<AppState>,
    Json(payload): Json<AuthVerifyRequest>,
) -> Result<Json<AuthVerifyResponse>, ApiError> {
    let wallet_address = normalize_wallet_address(&payload.wallet_address)?;
    let mut transaction = state.db.begin().await.map_err(internal_error)?;

    let wallet = sqlx::query_as::<_, WalletForVerification>(
        r#"
        SELECT id, user_id, nonce, nonce_expires_at
        FROM wallets
        WHERE address = $1
        FOR UPDATE
        "#,
    )
    .bind(&wallet_address)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invalid auth challenge"))?;

    if wallet.nonce.as_deref() != Some(payload.message.as_str()) {
        return Err(bad_request("Invalid auth challenge"));
    }

    let expires_at = wallet
        .nonce_expires_at
        .ok_or_else(|| bad_request("Invalid auth challenge"))?;

    if expires_at < Utc::now() {
        return Err(bad_request("Auth challenge expired"));
    }

    verify_wallet_signature(&wallet_address, &payload.message, &payload.signature)?;

    let user = if let Some(user_id) = wallet.user_id {
        sqlx::query_as::<_, User>(
            r#"
            UPDATE users
            SET name = COALESCE($2, name),
                email = COALESCE($3, email),
                designation = COALESCE($4, designation),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, email, designation, created_at, updated_at
            "#,
        )
        .bind(user_id)
        .bind(payload.name.as_deref())
        .bind(payload.email.as_deref())
        .bind(payload.designation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    } else {
        sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (name, email, designation)
            VALUES ($1, $2, $3)
            RETURNING id, name, email, designation, created_at, updated_at
            "#,
        )
        .bind(payload.name.as_deref())
        .bind(payload.email.as_deref())
        .bind(payload.designation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    };

    let updated_wallet = sqlx::query_as::<_, Wallet>(
        r#"
        UPDATE wallets
        SET user_id = $2,
            verified_at = NOW(),
            nonce = NULL,
            nonce_expires_at = NULL,
            updated_at = NOW()
        WHERE id = $1
        RETURNING id, address, label, user_id, verified_at, nonce,
                  nonce_expires_at, created_at, updated_at
        "#,
    )
    .bind(wallet.id)
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;

    transaction.commit().await.map_err(internal_error)?;

    Ok(Json(AuthVerifyResponse {
        user,
        wallet: updated_wallet,
    }))
}
