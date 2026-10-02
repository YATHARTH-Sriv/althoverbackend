use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub(super) struct ReceiptRow {
    pub id: Uuid,
    pub settings_account_id: Uuid,
    pub smart_account_id: Option<Uuid>,
    pub transaction_record_id: Option<Uuid>,
    pub uploaded_by_wallet: String,
    pub file_pathname: String,
    pub file_name: Option<String>,
    pub file_mime_type: Option<String>,
    pub file_size_bytes: Option<i32>,
    pub vendor: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    pub invoice_number: Option<String>,
    pub category: Option<String>,
    pub payment_recipient: Option<String>,
    pub confidence: Option<f64>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub workspace_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicReceipt {
    pub(super) id: Uuid,
    pub(super) settings_account_id: Uuid,
    pub(super) smart_account_id: Option<Uuid>,
    pub(super) transaction_record_id: Option<Uuid>,
    pub(super) uploaded_by_wallet: String,
    pub(super) file_url: String,
    pub(super) file_name: Option<String>,
    pub(super) file_mime_type: Option<String>,
    pub(super) file_size_bytes: Option<i32>,
    pub(super) vendor: Option<String>,
    pub(super) amount: Option<String>,
    pub(super) currency: Option<String>,
    pub(super) due_date: Option<DateTime<Utc>>,
    pub(super) invoice_number: Option<String>,
    pub(super) category: Option<String>,
    pub(super) payment_recipient: Option<String>,
    pub(super) confidence: Option<f64>,
    pub(super) status: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReceiptResponse {
    pub(super) receipt: PublicReceipt,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReceiptQuery {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReceiptRequest {
    pub(super) wallet_address: String,
    pub(super) smart_account_id: Option<Uuid>,
    pub(super) vendor: Option<String>,
    pub(super) amount: Option<serde_json::Value>,
    pub(super) currency: Option<String>,
    pub(super) due_date: Option<String>,
    pub(super) invoice_number: Option<String>,
    pub(super) category: Option<String>,
    pub(super) payment_recipient: Option<String>,
    pub(super) confidence: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReceiptRequest {
    pub(super) wallet_address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReceiptResponse {
    pub(super) ok: bool,
    pub(super) receipt_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptFileQuery {
    pub(super) wallet_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptPaymentRequest {
    pub(super) wallet_address: String,
    pub(super) transaction_record_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeProcessedReceipt {
    pub source_mime_type: String,
    pub extraction_mime_type: String,
    pub extraction: ExtractedReceipt,
    pub file: NodeFile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExtractedReceipt {
    pub vendor: Option<String>,
    pub amount: Option<serde_json::Value>,
    pub currency: Option<String>,
    pub due_date: Option<String>,
    pub invoice_number: Option<String>,
    pub category: Option<String>,
    pub payment_recipient: Option<String>,
    pub confidence: Option<f64>,
    pub raw_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeFile {
    pub url: String,
    pub pathname: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: usize,
}
