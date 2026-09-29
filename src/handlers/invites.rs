use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::auth::{normalize_wallet_address, verify_wallet_signature};

const INVITE_CHALLENGE_TTL_MINUTES: i64 = 10;
const INVITE_SELECT: &str = r#"
    SELECT invite.id, invite.workspace_id, invite.settings_account_id,
           invite.token, invite.status, invite.wallet_address, invite.email,
           invite.name, invite.designation, invite.permissions_mask,
           invite.expires_at, invite.accepted_at,
           workspace.name AS workspace_name, workspace.workspace_type,
           settings.name AS settings_name, settings.pda AS settings_pda,
           settings.threshold AS settings_threshold,
           settings.time_lock AS settings_time_lock
    FROM workspace_invites AS invite
    JOIN workspaces AS workspace ON workspace.id = invite.workspace_id
    JOIN settings_accounts AS settings ON settings.id = invite.settings_account_id
    WHERE invite.token = $1
"#;
const INVITE_SELECT_FOR_UPDATE: &str = r#"
    SELECT invite.id, invite.workspace_id, invite.settings_account_id,
           invite.token, invite.status, invite.wallet_address, invite.email,
           invite.name, invite.designation, invite.permissions_mask,
           invite.expires_at, invite.accepted_at,
           workspace.name AS workspace_name, workspace.workspace_type,
           settings.name AS settings_name, settings.pda AS settings_pda,
           settings.threshold AS settings_threshold,
           settings.time_lock AS settings_time_lock
    FROM workspace_invites AS invite
    JOIN workspaces AS workspace ON workspace.id = invite.workspace_id
    JOIN settings_accounts AS settings ON settings.id = invite.settings_account_id
    WHERE invite.token = $1
    FOR UPDATE OF invite
"#;

#[derive(Debug, FromRow)]
struct InviteRow {
    id: Uuid,
    workspace_id: Uuid,
    settings_account_id: Uuid,
    token: String,
    status: String,
    wallet_address: String,
    email: Option<String>,
    name: Option<String>,
    designation: Option<String>,
    permissions_mask: i32,
    expires_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
    workspace_name: String,
    workspace_type: String,
    settings_name: String,
    settings_pda: String,
    settings_threshold: i32,
    settings_time_lock: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicWorkspace {
    id: Uuid,
    name: String,
    #[serde(rename = "type")]
    workspace_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicSettingsAccount {
    id: Uuid,
    name: String,
    pda: String,
    threshold: i32,
    time_lock: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInvite {
    token: String,
    status: String,
    wallet_address: String,
    email: Option<String>,
    name: Option<String>,
    designation: Option<String>,
    permissions_mask: i32,
    expires_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
    invite_url: String,
    workspace: PublicWorkspace,
    settings_account: PublicSettingsAccount,
}

#[derive(Debug, Serialize)]
pub struct InviteResponse {
    invite: PublicInvite,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteChallengeRequest {
    wallet_address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteChallengeResponse {
    wallet_address: String,
    message: String,
    nonce_expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptInviteRequest {
    wallet_address: String,
    message: String,
    signature: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptInviteResponse {
    invite: PublicInvite,
    dashboard_path: &'static str,
}

#[derive(Debug, FromRow)]
struct WalletChallengeRow {
    id: Uuid,
    user_id: Option<Uuid>,
    nonce: Option<String>,
    nonce_expires_at: Option<DateTime<Utc>>,
}

fn normalize_invite_token(token: &str) -> Result<&str, ApiError> {
    let token = token.trim();
    if token.len() != 48 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bad_request("Invite not found"));
    }
    Ok(token)
}

fn invite_url(token: &str) -> String {
    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "http://localhost:3000".to_owned());
    format!("{}/invite/{token}", frontend_url.trim_end_matches('/'))
}

fn public_invite(invite: InviteRow) -> PublicInvite {
    PublicInvite {
        invite_url: invite_url(&invite.token),
        token: invite.token,
        status: invite.status,
        wallet_address: invite.wallet_address,
        email: invite.email,
        name: invite.name,
        designation: invite.designation,
        permissions_mask: invite.permissions_mask,
        expires_at: invite.expires_at,
        accepted_at: invite.accepted_at,
        workspace: PublicWorkspace {
            id: invite.workspace_id,
            name: invite.workspace_name,
            workspace_type: invite.workspace_type,
        },
        settings_account: PublicSettingsAccount {
            id: invite.settings_account_id,
            name: invite.settings_name,
            pda: invite.settings_pda,
            threshold: invite.settings_threshold,
            time_lock: invite.settings_time_lock,
        },
    }
}

async fn expire_invite_if_needed(state: &AppState, token: &str) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        UPDATE workspace_invites
        SET status = 'EXPIRED', updated_at = NOW()
        WHERE token = $1 AND status = 'PENDING' AND expires_at < NOW()
        "#,
    )
    .bind(token)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(())
}

async fn load_invite(state: &AppState, token: &str) -> Result<InviteRow, ApiError> {
    expire_invite_if_needed(state, token).await?;
    sqlx::query_as::<_, InviteRow>(INVITE_SELECT)
        .bind(token)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| bad_request("Invite not found"))
}

async fn load_invite_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    token: &str,
) -> Result<InviteRow, ApiError> {
    sqlx::query_as::<_, InviteRow>(INVITE_SELECT_FOR_UPDATE)
        .bind(token)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| bad_request("Invite not found"))
}

fn ensure_pending_invite(invite: &InviteRow) -> Result<(), ApiError> {
    if invite.status != "PENDING" {
        return Err(bad_request(format!(
            "Invite is {}",
            invite.status.to_lowercase()
        )));
    }
    Ok(())
}

fn generate_nonce() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(internal_error)?;
    Ok(hex::encode(bytes))
}

fn invite_challenge_message(invite: &InviteRow, wallet_address: &str, nonce: &str) -> String {
    format!(
        "Hover Agent invite acceptance\n\nWorkspace: {}\nInvite: {}\nWallet: {}\nNonce: {}",
        invite.workspace_name, invite.token, wallet_address, nonce
    )
}

fn validate_invite_challenge_message(
    invite: &InviteRow,
    wallet_address: &str,
    message: &str,
) -> Result<(), ApiError> {
    let prefix = format!(
        "Hover Agent invite acceptance\n\nWorkspace: {}\nInvite: {}\nWallet: {}\nNonce: ",
        invite.workspace_name, invite.token, wallet_address
    );
    let nonce = message
        .strip_prefix(&prefix)
        .ok_or_else(|| bad_request("Invalid invite challenge"))?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bad_request("Invalid invite challenge"));
    }
    Ok(())
}

pub async fn get_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<InviteResponse>, ApiError> {
    let token = normalize_invite_token(&token)?;
    let invite = load_invite(&state, token).await?;
    Ok(Json(InviteResponse {
        invite: public_invite(invite),
    }))
}

pub async fn create_invite_challenge(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(payload): Json<InviteChallengeRequest>,
) -> Result<Json<InviteChallengeResponse>, ApiError> {
    let token = normalize_invite_token(&token)?;
    let wallet_address = normalize_wallet_address(&payload.wallet_address)?;
    let invite = load_invite(&state, token).await?;
    ensure_pending_invite(&invite)?;
    if invite.wallet_address != wallet_address {
        return Err(bad_request(
            "Connect the wallet address this invite was created for",
        ));
    }

    let nonce = generate_nonce()?;
    let message = invite_challenge_message(&invite, &wallet_address, &nonce);
    let nonce_expires_at = Utc::now() + Duration::minutes(INVITE_CHALLENGE_TTL_MINUTES);
    sqlx::query(
        r#"
        INSERT INTO wallets (address, nonce, nonce_expires_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (address)
        DO UPDATE SET nonce = EXCLUDED.nonce,
                      nonce_expires_at = EXCLUDED.nonce_expires_at,
                      updated_at = NOW()
        "#,
    )
    .bind(&wallet_address)
    .bind(&message)
    .bind(nonce_expires_at)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(InviteChallengeResponse {
        wallet_address,
        message,
        nonce_expires_at,
    }))
}

pub async fn accept_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(payload): Json<AcceptInviteRequest>,
) -> Result<Json<AcceptInviteResponse>, ApiError> {
    let token = normalize_invite_token(&token)?.to_owned();
    let wallet_address = normalize_wallet_address(&payload.wallet_address)?;
    expire_invite_if_needed(&state, &token).await?;
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    let invite = load_invite_for_update(&mut transaction, &token).await?;
    ensure_pending_invite(&invite)?;
    if invite.wallet_address != wallet_address {
        return Err(bad_request(
            "Connected wallet does not match the invited signer wallet",
        ));
    }
    validate_invite_challenge_message(&invite, &wallet_address, &payload.message)?;

    let wallet = sqlx::query_as::<_, WalletChallengeRow>(
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
    .ok_or_else(|| bad_request("Invalid invite challenge"))?;

    if wallet.nonce.as_deref() != Some(payload.message.as_str()) {
        return Err(bad_request("Invalid invite challenge"));
    }
    if wallet
        .nonce_expires_at
        .is_none_or(|expiry| expiry < Utc::now())
    {
        return Err(bad_request("Invite challenge expired"));
    }
    verify_wallet_signature(&wallet_address, &payload.message, &payload.signature)?;

    let user_id = if let Some(user_id) = wallet.user_id {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            UPDATE users
            SET name = COALESCE($2, name),
                email = COALESCE($3, email),
                designation = COALESCE($4, designation),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(invite.name.as_deref())
        .bind(invite.email.as_deref())
        .bind(invite.designation.as_deref())
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
        .bind(invite.name.as_deref())
        .bind(invite.email.as_deref())
        .bind(invite.designation.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?
    };

    let label = invite
        .name
        .as_deref()
        .or(invite.designation.as_deref())
        .unwrap_or("Signer");
    let consumed = sqlx::query(
        r#"
        UPDATE wallets
        SET user_id = $2, verified_at = NOW(), nonce = NULL,
            nonce_expires_at = NULL, label = $3, updated_at = NOW()
        WHERE id = $1 AND nonce = $4 AND nonce_expires_at > NOW()
        "#,
    )
    .bind(wallet.id)
    .bind(user_id)
    .bind(label)
    .bind(&payload.message)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if consumed.rows_affected() != 1 {
        return Err(bad_request("Invite challenge was already used"));
    }

    sqlx::query(
        r#"
        INSERT INTO workspace_members (workspace_id, user_id, role)
        VALUES ($1, $2, 'MEMBER')
        ON CONFLICT (workspace_id, user_id)
        DO UPDATE SET role = 'MEMBER', updated_at = NOW()
        "#,
    )
    .bind(invite.workspace_id)
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    sqlx::query(
        r#"
        INSERT INTO settings_signers (
            settings_account_id, wallet_address, permissions_mask, role, label
        )
        VALUES ($1, $2, $3, 'EXTERNAL', $4)
        ON CONFLICT (settings_account_id, wallet_address)
        DO UPDATE SET permissions_mask = EXCLUDED.permissions_mask,
                      role = 'EXTERNAL', label = EXCLUDED.label,
                      updated_at = NOW()
        "#,
    )
    .bind(invite.settings_account_id)
    .bind(&wallet_address)
    .bind(invite.permissions_mask)
    .bind(label)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    sqlx::query(
        r#"
        UPDATE workspace_invites
        SET status = 'ACCEPTED', accepted_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND status = 'PENDING'
        "#,
    )
    .bind(invite.id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    let description = invite.name.as_deref().map_or_else(
        || "A signer joined the workspace".to_owned(),
        |name| format!("{name} joined the workspace"),
    );
    sqlx::query(
        r#"
        INSERT INTO activity_logs (
            user_id, workspace_id, settings_account_id, activity_type,
            title, description, metadata
        )
        VALUES ($1, $2, $3, 'INVITE_ACCEPTED',
                'Signer invite accepted', $4, $5)
        "#,
    )
    .bind(user_id)
    .bind(invite.workspace_id)
    .bind(invite.settings_account_id)
    .bind(description)
    .bind(serde_json::json!({
        "walletAddress": wallet_address,
        "inviteToken": invite.token,
    }))
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    transaction.commit().await.map_err(internal_error)?;
    let accepted_invite = load_invite(&state, &token).await?;
    Ok(Json(AcceptInviteResponse {
        invite: public_invite(accepted_invite),
        dashboard_path: "/dashboard",
    }))
}

#[cfg(test)]
mod tests {
    use super::normalize_invite_token;

    #[test]
    fn accepts_the_generated_invite_token_shape() {
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(matches!(normalize_invite_token(token), Ok(value) if value == token));
    }

    #[test]
    fn rejects_malformed_invite_tokens() {
        assert!(normalize_invite_token("short").is_err());
        assert!(normalize_invite_token(&"g".repeat(48)).is_err());
    }
}
