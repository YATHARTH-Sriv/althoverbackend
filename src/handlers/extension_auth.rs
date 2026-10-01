use std::str::FromStr;

use axum::{Json, extract::State, http::HeaderMap};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::auth::{normalize_wallet_address, verify_wallet_signature};

const EXTENSION_SESSION_TTL_DAYS: i64 = 30;
const EXTENSION_EXCHANGE_TTL_MINUTES: i64 = 10;
const DEFAULT_SESSION_NAME: &str = "Hover Capture";
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExtensionSessionRequest {
    wallet_address: String,
    settings_pda: String,
    message: String,
    signature: String,
    code_challenge: String,
    extension_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeExtensionSessionRequest {
    code: String,
    code_verifier: String,
}

#[derive(Debug, FromRow)]
struct WalletChallengeRow {
    nonce: Option<String>,
    nonce_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct ExtensionSettingsRow {
    id: Uuid,
    name: String,
    pda: String,
}

#[derive(Debug, FromRow)]
struct PendingSessionRow {
    id: Uuid,
    wallet_address: String,
    settings_account_id: Uuid,
    code_challenge: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct ActiveSessionRow {
    id: Uuid,
    wallet_address: String,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    wallet_label: Option<String>,
    user_name: Option<String>,
    settings_id: Uuid,
    settings_name: String,
    settings_pda: String,
    settings_threshold: i32,
    workspace_id: Uuid,
    workspace_name: String,
    workspace_type: String,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionSmartAccount {
    id: Uuid,
    name: String,
    pda: String,
    account_index: i32,
    balance_lamports: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionWorkspace {
    id: Uuid,
    name: String,
    #[serde(rename = "type")]
    workspace_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionSettings {
    id: Uuid,
    name: String,
    pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrimarySmartAccount {
    id: Uuid,
    name: String,
    pda: String,
    balance_lamports: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExtensionSessionResponse {
    code: String,
    expires_at: DateTime<Utc>,
    wallet_address: String,
    settings: ExtensionSettings,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeExtensionSessionResponse {
    token: String,
    expires_at: DateTime<Utc>,
    wallet_address: String,
    workspace: ExtensionWorkspace,
    settings: ExtensionSettings,
    smart_account: Option<PrimarySmartAccount>,
    smart_accounts: Vec<ExtensionSmartAccount>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetExtensionSessionResponse {
    connected: bool,
    wallet_address: String,
    wallet_label: Option<String>,
    expires_at: DateTime<Utc>,
    workspace: ExtensionWorkspace,
    settings: ExtensionSettings,
    smart_account: Option<PrimarySmartAccount>,
    smart_accounts: Vec<ExtensionSmartAccount>,
}

#[derive(Debug, Serialize)]
pub struct RevokeExtensionSessionResponse {
    ok: bool,
}

fn required_trimmed(value: &str, field: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request(format!("{field} is required")));
    }
    Ok(value.to_owned())
}

fn validate_code_challenge(value: Option<&str>) -> Result<String, ApiError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_request("codeChallenge is required"))?;
    if value.len() != 43
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(bad_request(
            "codeChallenge must be a SHA-256 PKCE challenge",
        ));
    }
    Ok(value.to_owned())
}

fn normalize_extension_id(
    value: Option<&str>,
    allowed_extension_ids: &[String],
) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() != 32 || !value.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
        return Err(bad_request("extensionId is invalid"));
    }

    if !allowed_extension_ids.is_empty()
        && !allowed_extension_ids
            .iter()
            .any(|allowed_id| allowed_id == value)
    {
        return Err(bad_request("extensionId is not allowed"));
    }

    Ok(Some(value.to_owned()))
}

pub(crate) fn extension_challenge_context(
    purpose: Option<&str>,
    settings_pda: Option<&str>,
    code_challenge: Option<&str>,
    extension_id: Option<&str>,
    allowed_extension_ids: &[String],
) -> Result<String, ApiError> {
    if purpose.map(str::trim) != Some("extension-session") {
        return Ok(String::new());
    }

    let settings_pda = settings_pda
        .ok_or_else(|| bad_request("settingsPda is required"))?
        .trim();
    let settings_pda = Pubkey::from_str(settings_pda)
        .map_err(|_| bad_request("settingsPda is invalid"))?
        .to_string();
    let code_challenge = validate_code_challenge(code_challenge)?;
    let extension_id = normalize_extension_id(extension_id, allowed_extension_ids)?
        .unwrap_or_else(|| "unpacked-development".to_owned());

    Ok(format!(
        "\n  Purpose: extension-session\n  Settings: {settings_pda}\n  PKCE Challenge: {code_challenge}\n  Extension ID: {extension_id}"
    ))
}

fn generate_secret(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(internal_error)?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes)))
}

fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn extension_exchange_code_hash(code: &str) -> String {
    sha256_hex(&format!("hover-extension-code:{code}"))
}

pub(crate) fn extension_token_hash(state: &AppState, token: &str) -> Result<String, ApiError> {
    let pepper = state
        .config
        .extension_token_pepper
        .as_deref()
        .ok_or_else(|| internal_error("EXTENSION_TOKEN_PEPPER is required"))?;
    Ok(sha256_hex(&format!("{}:{token}", pepper)))
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn bearer_token(headers: &HeaderMap) -> Result<String, ApiError> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut parts = authorization.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(scheme), Some(token), None) if scheme.eq_ignore_ascii_case("Bearer") => {
            required_trimmed(token, "Extension session token")
        }
        _ => Err(bad_request("Extension session token is required")),
    }
}

async fn load_smart_accounts(
    state: &AppState,
    settings_id: Uuid,
) -> Result<Vec<ExtensionSmartAccount>, ApiError> {
    sqlx::query_as::<_, ExtensionSmartAccount>(
        r#"
        SELECT id, name, pda, account_index,
               balance_lamports::text AS balance_lamports
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

fn primary_smart_account(accounts: &[ExtensionSmartAccount]) -> Option<PrimarySmartAccount> {
    accounts.first().map(|account| PrimarySmartAccount {
        id: account.id,
        name: account.name.clone(),
        pda: account.pda.clone(),
        balance_lamports: account.balance_lamports.clone(),
    })
}

pub async fn create_extension_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateExtensionSessionRequest>,
) -> Result<Json<CreateExtensionSessionResponse>, ApiError> {
    let wallet_address = normalize_wallet_address(&payload.wallet_address)?;
    let settings_pda = Pubkey::from_str(payload.settings_pda.trim())
        .map_err(|_| bad_request("settingsPda is invalid"))?
        .to_string();
    let message = required_trimmed(&payload.message, "message")?;
    let code_challenge = validate_code_challenge(Some(&payload.code_challenge))?;
    let extension_id =
        normalize_extension_id(payload.extension_id.as_deref(), &state.config.extension_ids)?;
    let extension_label = extension_id
        .clone()
        .unwrap_or_else(|| "unpacked-development".to_owned());

    let wallet = sqlx::query_as::<_, WalletChallengeRow>(
        "SELECT nonce, nonce_expires_at FROM wallets WHERE address = $1",
    )
    .bind(&wallet_address)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invalid auth challenge"))?;

    if wallet.nonce.as_deref() != Some(message.as_str()) {
        return Err(bad_request("Invalid auth challenge"));
    }
    if wallet
        .nonce_expires_at
        .is_none_or(|expires| expires <= Utc::now())
    {
        return Err(bad_request("Auth challenge expired"));
    }
    verify_wallet_signature(&wallet_address, &message, &payload.signature)?;

    let expected_context = format!(
        "Purpose: extension-session\n  Settings: {settings_pda}\n  PKCE Challenge: {code_challenge}\n  Extension ID: {extension_label}"
    );
    if !message.contains(&expected_context) {
        return Err(bad_request(
            "Auth challenge is not bound to this extension authorization",
        ));
    }

    let settings = sqlx::query_as::<_, ExtensionSettingsRow>(
        r#"
        SELECT settings.id, settings.name, settings.pda
        FROM settings_accounts AS settings
        WHERE settings.pda = $1
          AND EXISTS (
              SELECT 1 FROM settings_signers AS signer
              WHERE signer.settings_account_id = settings.id
                AND signer.wallet_address = $2
          )
        "#,
    )
    .bind(&settings_pda)
    .bind(&wallet_address)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Wallet is not a signer for this settings account"))?;

    let exchange_code = generate_secret("hvr_code_")?;
    let exchange_code_hash = extension_exchange_code_hash(&exchange_code);
    let exchange_expires_at = Utc::now() + Duration::minutes(EXTENSION_EXCHANGE_TTL_MINUTES);
    let expires_at = Utc::now() + Duration::days(EXTENSION_SESSION_TTL_DAYS);
    let session_name = payload
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_SESSION_NAME);
    let mut transaction = state.db.begin().await.map_err(internal_error)?;

    let consumed = sqlx::query(
        r#"
        UPDATE wallets
        SET verified_at = NOW(), nonce = NULL, nonce_expires_at = NULL,
            updated_at = NOW()
        WHERE address = $1 AND nonce = $2 AND nonce_expires_at > NOW()
        "#,
    )
    .bind(&wallet_address)
    .bind(&message)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if consumed.rows_affected() != 1 {
        return Err(bad_request("Auth challenge was already used"));
    }

    sqlx::query(
        r#"
        INSERT INTO extension_sessions (
            exchange_code_hash, code_challenge, extension_id, wallet_address,
            settings_account_id, name, exchange_expires_at, expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(exchange_code_hash)
    .bind(&code_challenge)
    .bind(extension_id.as_deref())
    .bind(&wallet_address)
    .bind(settings.id)
    .bind(session_name)
    .bind(exchange_expires_at)
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;

    transaction.commit().await.map_err(internal_error)?;

    Ok(Json(CreateExtensionSessionResponse {
        code: exchange_code,
        expires_at,
        wallet_address,
        settings: ExtensionSettings {
            id: settings.id,
            name: settings.name,
            pda: settings.pda,
            threshold: None,
        },
    }))
}

pub async fn exchange_extension_session(
    State(state): State<AppState>,
    Json(payload): Json<ExchangeExtensionSessionRequest>,
) -> Result<Json<ExchangeExtensionSessionResponse>, ApiError> {
    let code = required_trimmed(&payload.code, "code")?;
    let verifier = required_trimmed(&payload.code_verifier, "codeVerifier")?;
    if !(43..=128).contains(&verifier.len()) {
        return Err(bad_request(
            "codeVerifier must be between 43 and 128 characters",
        ));
    }
    let exchange_hash = extension_exchange_code_hash(&code);
    let session = sqlx::query_as::<_, PendingSessionRow>(
        r#"
        SELECT id, wallet_address, settings_account_id, code_challenge, expires_at
        FROM extension_sessions
        WHERE exchange_code_hash = $1 AND exchanged_at IS NULL
          AND revoked_at IS NULL AND exchange_expires_at > NOW()
          AND expires_at > NOW()
        "#,
    )
    .bind(&exchange_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Extension authorization expired. Connect Hover again."))?;

    if pkce_challenge(&verifier) != session.code_challenge {
        return Err(bad_request("Extension authorization verifier is invalid"));
    }

    let token = generate_secret("hvr_ext_")?;
    let token_hash = extension_token_hash(&state, &token)?;
    let claimed = sqlx::query(
        r#"
        UPDATE extension_sessions
        SET token_hash = $2, exchanged_at = NOW(), exchange_code_hash = NULL,
            last_used_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND exchanged_at IS NULL AND exchange_code_hash = $3
        "#,
    )
    .bind(session.id)
    .bind(token_hash)
    .bind(&exchange_hash)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    if claimed.rows_affected() != 1 {
        return Err(bad_request("Extension authorization was already used"));
    }

    let context = load_active_session_by_id(&state, session.id).await?;
    let smart_accounts = load_smart_accounts(&state, session.settings_account_id).await?;
    let smart_account = primary_smart_account(&smart_accounts);

    Ok(Json(ExchangeExtensionSessionResponse {
        token,
        expires_at: session.expires_at,
        wallet_address: session.wallet_address,
        workspace: workspace_response(&context),
        settings: settings_response(&context),
        smart_account,
        smart_accounts,
    }))
}

async fn load_active_session_by_id(
    state: &AppState,
    session_id: Uuid,
) -> Result<ActiveSessionRow, ApiError> {
    sqlx::query_as::<_, ActiveSessionRow>(
        r#"
        SELECT session.id, session.wallet_address, session.expires_at,
               session.revoked_at, wallet.label AS wallet_label,
               app_user.name AS user_name,
               settings.id AS settings_id, settings.name AS settings_name,
               settings.pda AS settings_pda, settings.threshold AS settings_threshold,
               workspace.id AS workspace_id, workspace.name AS workspace_name,
               workspace.workspace_type
        FROM extension_sessions AS session
        JOIN settings_accounts AS settings ON settings.id = session.settings_account_id
        JOIN workspaces AS workspace ON workspace.id = settings.workspace_id
        JOIN wallets AS wallet ON wallet.address = session.wallet_address
        LEFT JOIN users AS app_user ON app_user.id = wallet.user_id
        WHERE session.id = $1
        "#,
    )
    .bind(session_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)
}

fn workspace_response(session: &ActiveSessionRow) -> ExtensionWorkspace {
    ExtensionWorkspace {
        id: session.workspace_id,
        name: session.workspace_name.clone(),
        workspace_type: session.workspace_type.clone(),
    }
}

fn settings_response(session: &ActiveSessionRow) -> ExtensionSettings {
    ExtensionSettings {
        id: session.settings_id,
        name: session.settings_name.clone(),
        pda: session.settings_pda.clone(),
        threshold: Some(session.settings_threshold),
    }
}

pub async fn get_extension_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<GetExtensionSessionResponse>, ApiError> {
    let token = bearer_token(&headers)?;
    let token_hash = extension_token_hash(&state, &token)?;
    let session_id =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM extension_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(&state.db)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| bad_request("Extension session is not active"))?;
    let session = load_active_session_by_id(&state, session_id).await?;

    if session.revoked_at.is_some() {
        return Err(bad_request("Extension session is not active"));
    }
    if session.expires_at <= Utc::now() {
        return Err(bad_request(
            "Extension session expired. Connect Hover again.",
        ));
    }

    sqlx::query(
        "UPDATE extension_sessions SET last_used_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(session.id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    let smart_accounts = load_smart_accounts(&state, session.settings_id).await?;
    let smart_account = primary_smart_account(&smart_accounts);

    Ok(Json(GetExtensionSessionResponse {
        connected: true,
        wallet_address: session.wallet_address.clone(),
        wallet_label: session.wallet_label.clone().or(session.user_name.clone()),
        expires_at: session.expires_at,
        workspace: workspace_response(&session),
        settings: settings_response(&session),
        smart_account,
        smart_accounts,
    }))
}

pub async fn revoke_extension_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RevokeExtensionSessionResponse>, ApiError> {
    let token = bearer_token(&headers)?;
    let token_hash = extension_token_hash(&state, &token)?;
    sqlx::query(
        r#"
        UPDATE extension_sessions
        SET revoked_at = NOW(), updated_at = NOW()
        WHERE token_hash = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(token_hash)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(Json(RevokeExtensionSessionResponse { ok: true }))
}

#[cfg(test)]
mod tests {
    use super::{extension_challenge_context, pkce_challenge};

    #[test]
    fn creates_the_expected_pkce_challenge() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn rejects_a_short_pkce_challenge() {
        assert!(
            extension_challenge_context(
                Some("extension-session"),
                Some("11111111111111111111111111111111"),
                Some("short"),
                None,
                &[],
            )
            .is_err()
        );
    }
}
