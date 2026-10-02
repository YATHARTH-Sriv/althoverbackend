use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineItemInput {
    pub(super) description: String,
    pub(super) quantity: String,
    pub(super) unit_price: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvoiceRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) customer_id: Uuid,
    pub(super) receiving_smart_account_id: Uuid,
    pub(super) issue_date: String,
    pub(super) due_date: Option<String>,
    pub(super) memo: Option<String>,
    pub(super) line_items: Vec<InvoiceLineItemInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInvoiceRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) customer_id: Option<Uuid>,
    pub(super) receiving_smart_account_id: Option<Uuid>,
    pub(super) issue_date: Option<String>,
    pub(super) due_date: Option<String>,
    pub(super) memo: Option<String>,
    pub(super) line_items: Option<Vec<InvoiceLineItemInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceAccessQuery {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInvoicesQuery {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
    pub(super) status: Option<String>,
    pub(super) limit: Option<i64>,
    pub(super) offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct InvoiceRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub customer_id: Uuid,
    pub receiving_smart_account_id: Uuid,
    pub invoice_number: i64,
    pub status: String,
    pub currency: String,
    pub issue_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    pub customer_business_name: String,
    pub customer_billing_email: Option<String>,
    pub customer_contact_name: Option<String>,
    pub customer_wallet_address: Option<String>,
    pub memo: Option<String>,
    pub subtotal: String,
    pub total: String,
    pub amount_paid: String,
    pub created_by_wallet: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub viewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct InvoiceLineItemResponse {
    pub id: Uuid,
    pub position: i32,
    pub description: String,
    pub quantity: String,
    pub unit_price: String,
    pub amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceResponse {
    #[serde(flatten)]
    pub(super) invoice: InvoiceRow,
    pub(super) line_items: Vec<InvoiceLineItemResponse>,
    pub(super) payments: Vec<InvoicePaymentResponse>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct InvoicePaymentResponse {
    pub id: Uuid,
    pub payer_wallet: String,
    pub amount_lamports: String,
    pub tx_sig: String,
    pub confirmed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendInvoiceRequest {
    pub(super) wallet_address: String,
    pub(super) settings_pda: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendInvoiceResponse {
    pub(super) invoice: InvoiceResponse,
    pub(super) public_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInvoicePaymentRequest {
    pub(super) payer_wallet: String,
    pub(super) amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInvoicePaymentResponse {
    pub(super) transaction_base64: String,
    pub(super) recipient: String,
    pub(super) amount_lamports: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoicePaymentSubmittedRequest {
    pub(super) payer_wallet: String,
    pub(super) amount_lamports: String,
    pub(super) tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInvoiceResponse {
    pub(super) invoice: InvoiceResponse,
    pub(super) workspace_name: String,
    pub(super) smart_account_name: String,
    pub(super) recipient: String,
}

#[derive(Debug, Serialize)]
pub struct InvoiceItemResponse {
    pub(super) invoice: InvoiceResponse,
}

#[derive(Debug, Serialize)]
pub struct InvoiceListResponse {
    pub(super) invoices: Vec<InvoiceResponse>,
}

#[derive(Debug, FromRow)]
pub(super) struct CustomerSnapshot {
    pub business_name: String,
    pub billing_email: Option<String>,
    pub contact_name: Option<String>,
    pub wallet_address: Option<String>,
}

pub(super) struct PreparedLineItem {
    pub description: String,
    pub quantity_units: i128,
    pub unit_price_units: i128,
    pub amount_units: i128,
}
