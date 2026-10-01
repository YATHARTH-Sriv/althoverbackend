use sqlx::FromRow;
use uuid::Uuid;

use crate::{ApiError, AppState, internal_error, unauthorized, validation::parse_pubkey};

#[derive(Debug, Clone, FromRow)]
pub struct SettingsAccess {
    pub settings_account_id: Uuid,
    pub workspace_id: Uuid,
    pub settings_authority: String,
    pub permissions_mask: i32,
}

pub async fn require_settings_access(
    state: &AppState,
    settings_pda: &str,
    wallet_address: &str,
    required_permission: Option<i32>,
    authority_only: bool,
) -> Result<SettingsAccess, ApiError> {
    let settings_pda = parse_pubkey(settings_pda, "settingsPda")?.to_string();
    let wallet_address = parse_pubkey(wallet_address, "walletAddress")?.to_string();
    let access = sqlx::query_as::<_, SettingsAccess>(
        r#"
        SELECT settings.id AS settings_account_id, settings.workspace_id,
               settings.settings_authority,
               COALESCE(signer.permissions_mask, 0) AS permissions_mask
        FROM settings_accounts AS settings
        LEFT JOIN settings_signers AS signer
          ON signer.settings_account_id = settings.id
         AND signer.wallet_address = $2
        WHERE settings.pda = $1
          AND (settings.settings_authority = $2 OR signer.wallet_address IS NOT NULL)
        "#,
    )
    .bind(settings_pda)
    .bind(&wallet_address)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| unauthorized("Wallet is not a signer for this settings account"))?;

    if authority_only && access.settings_authority != wallet_address {
        return Err(unauthorized(
            "Only the Settings authority can perform this action",
        ));
    }
    if let Some(permission) = required_permission
        && access.settings_authority != wallet_address
        && access.permissions_mask & permission == 0
    {
        return Err(unauthorized("Wallet does not have the required permission"));
    }
    Ok(access)
}

pub async fn require_settings_member_by_id(
    state: &AppState,
    settings_account_id: Uuid,
    wallet_address: &str,
) -> Result<(), ApiError> {
    let wallet_address = parse_pubkey(wallet_address, "walletAddress")?.to_string();
    let allowed = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM settings_accounts AS settings
            LEFT JOIN settings_signers AS signer
              ON signer.settings_account_id = settings.id
             AND signer.wallet_address = $2
            WHERE settings.id = $1
              AND (settings.settings_authority = $2 OR signer.wallet_address IS NOT NULL)
        )
        "#,
    )
    .bind(settings_account_id)
    .bind(wallet_address)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    if !allowed {
        return Err(unauthorized(
            "Wallet is not a signer for this settings account",
        ));
    }
    Ok(())
}
