use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{ApiError, bad_request};

#[derive(Debug, FromRow)]
pub(super) struct SettingsForTransaction {
    pub(super) id: Uuid,
    pub(super) workspace_id: Uuid,
    pub(super) pda: String,
    pub(super) stale_transaction_index: String,
}

#[derive(Debug, FromRow)]
pub(super) struct ApprovalWithTransaction {
    pub(super) transaction_record_id: Uuid,
    pub(super) id: Uuid,
    pub(super) wallet_address: String,
    pub(super) tx_sig: Option<String>,
    pub(super) status: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
pub(super) struct PayoutWithTransaction {
    pub(super) transaction_record_id: Uuid,
    pub(super) payout_id: Uuid,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) category: String,
    pub(super) recipient_name: String,
    pub(super) recipient_memo: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct TransactionRow {
    pub(super) id: Uuid,
    pub(super) settings_account_id: Uuid,
    pub(super) smart_account_id: Uuid,
    pub(super) transaction_pda: String,
    pub(super) proposal_pda: String,
    pub(super) smart_account_pda: String,
    pub(super) transaction_index: String,
    pub(super) account_index: i32,
    pub(super) recipient: String,
    pub(super) amount_lamports: String,
    pub(super) memo: String,
    pub(super) status: String,
    pub(super) create_tx_sig: String,
    pub(super) proposal_tx_sig: String,
    pub(super) execute_tx_sig: Option<String>,
    pub(super) created_by_wallet: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApprovalResponse {
    pub(super) id: Uuid,
    pub(super) wallet_address: String,
    pub(super) tx_sig: Option<String>,
    pub(super) status: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PayoutContextResponse {
    pub(super) payout_id: Uuid,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) category: String,
    pub(super) recipient_name: String,
    pub(super) recipient_memo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionResponse {
    pub(super) id: Uuid,
    pub(super) settings_account_id: Uuid,
    pub(super) smart_account_id: Uuid,
    #[serde(rename = "type")]
    pub(super) transaction_type: &'static str,
    pub(super) transaction_pda: String,
    pub(super) proposal_pda: String,
    pub(super) smart_account_pda: String,
    pub(super) transaction_index: String,
    pub(super) account_index: i32,
    pub(super) recipient: String,
    pub(super) amount_lamports: String,
    pub(super) memo: String,
    pub(super) status: String,
    pub(super) create_tx_sig: String,
    pub(super) proposal_tx_sig: String,
    pub(super) execute_tx_sig: Option<String>,
    pub(super) created_by_wallet: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    pub(super) approvals: Vec<ApprovalResponse>,
    pub(super) is_stale: bool,
    pub(super) stale_reason: Option<&'static str>,
    pub(super) payout: Option<PayoutContextResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum IntegerInput {
    Text(String),
    Number(u64),
}

impl IntegerInput {
    pub(super) fn parse(self, field: &str, allow_zero: bool) -> Result<u64, ApiError> {
        let value = match self {
            Self::Text(value) => value.trim().parse::<u64>(),
            Self::Number(value) => Ok(value),
        }
        .map_err(|_| bad_request(format!("{field} must be an unsigned integer")))?;
        if !allow_zero && value == 0 {
            return Err(bad_request(format!("{field} must be greater than 0")));
        }
        Ok(value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateTransactionRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) recipient: String,
    pub(super) account_index: i32,
    pub(super) amount_lamports: IntegerInput,
    pub(super) memo: Option<String>,
    pub(super) smart_account_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildCreateTransactionResponse {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) account_index: u8,
    pub(super) recipient: String,
    pub(super) amount_lamports: String,
    pub(super) memo: String,
    pub(super) smart_account_name: Option<String>,
    pub(super) smart_account_pda: String,
    pub(super) transaction_pda: String,
    pub(super) transaction_index: String,
    pub(super) transaction_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProposalRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) transaction_index: IntegerInput,
    pub(super) draft: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProposalResponse {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) transaction_index: String,
    pub(super) proposal_pda: String,
    pub(super) transaction_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletSubmittedRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) recipient: String,
    pub(super) account_index: i32,
    pub(super) transaction_index: IntegerInput,
    pub(super) transaction_pda: String,
    pub(super) proposal_pda: String,
    pub(super) smart_account_pda: String,
    pub(super) amount_lamports: IntegerInput,
    pub(super) create_tx_sig: String,
    pub(super) proposal_tx_sig: String,
    pub(super) memo: Option<String>,
    pub(super) smart_account_name: Option<String>,
    pub(super) receipt_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletQuery {
    pub(super) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmittedActionRequest {
    pub(super) wallet_address: String,
    pub(super) tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltActionResponse {
    pub(super) transaction_id: Uuid,
    pub(super) wallet_address: String,
    pub(super) transaction_base64: String,
    pub(super) proposal_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) smart_account_pda: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransactionEnvelope {
    pub(super) transaction: TransactionResponse,
}

#[derive(Debug, Serialize)]
pub struct TransactionProposalEnvelope {
    pub(super) transaction: TransactionResponse,
    pub(super) proposal: ProposalResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalResponse {
    pub(super) settings: String,
    pub(super) transaction_index: String,
    pub(super) status: String,
    pub(super) approved: Vec<String>,
    pub(super) rejected: Vec<String>,
    pub(super) cancelled: Vec<String>,
}
