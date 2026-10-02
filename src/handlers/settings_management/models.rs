use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub(super) struct IndexedSettings {
    pub(super) id: Uuid,
    pub(super) workspace_id: Uuid,
    pub(super) pda: String,
}

#[derive(Debug, FromRow)]
pub(super) struct SettingsRow {
    pub(super) id: Uuid,
    pub(super) workspace_id: Uuid,
    pub(super) name: String,
    pub(super) pda: String,
    pub(super) seed: String,
    pub(super) settings_authority: String,
    pub(super) threshold: i32,
    pub(super) time_lock: i64,
    pub(super) transaction_index: String,
    pub(super) stale_transaction_index: String,
    pub(super) creation_tx_sig: Option<String>,
    pub(super) created_by_wallet: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSignerResponse {
    pub(super) id: Uuid,
    pub(super) wallet_address: String,
    pub(super) role: String,
    pub(super) permissions_mask: i32,
    pub(super) label: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    pub(super) id: Uuid,
    pub(super) name: String,
    #[serde(rename = "type")]
    pub(super) workspace_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    pub(super) id: Uuid,
    pub(super) workspace_id: Uuid,
    pub(super) name: String,
    pub(super) pda: String,
    pub(super) seed: String,
    pub(super) settings_authority: String,
    pub(super) threshold: i32,
    pub(super) time_lock: i64,
    pub(super) transaction_index: String,
    pub(super) stale_transaction_index: String,
    pub(super) creation_tx_sig: Option<String>,
    pub(super) created_by_wallet: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    pub(super) workspace: WorkspaceResponse,
    pub(super) signers: Vec<SettingsSignerResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeThresholdBuildRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) new_threshold: u16,
    pub(super) memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeThresholdSubmittedRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) new_threshold: u16,
    pub(super) tx_sig: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSignerBuildRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) signer: String,
    pub(super) permissions_mask: Option<u8>,
    pub(super) memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSignerSubmittedRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) signer: String,
    pub(super) permissions_mask: Option<u8>,
    pub(super) name: Option<String>,
    pub(super) email: Option<String>,
    pub(super) designation: Option<String>,
    pub(super) tx_sig: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSignerBuildRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) signer: String,
    pub(super) memo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSignerSubmittedRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) signer: String,
    pub(super) tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSettingsTransactionResponse {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) signer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) permissions_mask: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) new_threshold: Option<u16>,
    pub(super) memo: String,
    pub(super) transaction_base64: String,
}

#[derive(Debug, Serialize)]
pub struct SubmittedSettingsResponse {
    pub(super) settings: SettingsResponse,
}

#[derive(Debug, FromRow)]
pub(super) struct InviteRow {
    pub(super) token: String,
    pub(super) status: String,
    pub(super) wallet_address: String,
    pub(super) email: Option<String>,
    pub(super) name: Option<String>,
    pub(super) designation: Option<String>,
    pub(super) permissions_mask: i32,
    pub(super) expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteResponse {
    pub(super) token: String,
    pub(super) invite_url: String,
    pub(super) status: String,
    pub(super) wallet_address: String,
    pub(super) email: Option<String>,
    pub(super) name: Option<String>,
    pub(super) designation: Option<String>,
    pub(super) permissions_mask: i32,
    pub(super) expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AddSignerSubmittedResponse {
    pub(super) settings: SettingsResponse,
    pub(super) invite: InviteResponse,
}
