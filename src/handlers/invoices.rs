use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use solana_system_interface::instruction as system_instruction;
use sqlx::{FromRow, Postgres, Transaction};
use std::str::FromStr;
use uuid::Uuid;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    money::{format_units, parse_decimal_units},
    solanasetup::{build_unsigned_transaction_base64, verify_confirmed_sol_transfer},
};

use super::{auth::normalize_wallet_address, customers::customer_access};

const QUANTITY_SCALE: u32 = 6;
const MONEY_SCALE: u32 = 9;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineItemInput {
    description: String,
    quantity: String,
    unit_price: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvoiceRequest {
    wallet_address: String,
    settings_pda: String,
    customer_id: Uuid,
    receiving_smart_account_id: Uuid,
    issue_date: String,
    due_date: Option<String>,
    memo: Option<String>,
    line_items: Vec<InvoiceLineItemInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInvoiceRequest {
    wallet_address: String,
    settings_pda: String,
    customer_id: Option<Uuid>,
    receiving_smart_account_id: Option<Uuid>,
    issue_date: Option<String>,
    due_date: Option<String>,
    memo: Option<String>,
    line_items: Option<Vec<InvoiceLineItemInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceAccessQuery {
    wallet_address: String,
    settings_pda: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInvoicesQuery {
    wallet_address: String,
    settings_pda: String,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct InvoiceRow {
    id: Uuid,
    workspace_id: Uuid,
    customer_id: Uuid,
    receiving_smart_account_id: Uuid,
    invoice_number: i64,
    status: String,
    currency: String,
    issue_date: NaiveDate,
    due_date: Option<NaiveDate>,
    customer_business_name: String,
    customer_billing_email: Option<String>,
    customer_contact_name: Option<String>,
    customer_wallet_address: Option<String>,
    memo: Option<String>,
    subtotal: String,
    total: String,
    amount_paid: String,
    created_by_wallet: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
    viewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct InvoiceLineItemResponse {
    id: Uuid,
    position: i32,
    description: String,
    quantity: String,
    unit_price: String,
    amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceResponse {
    #[serde(flatten)]
    invoice: InvoiceRow,
    line_items: Vec<InvoiceLineItemResponse>,
    payments: Vec<InvoicePaymentResponse>,
}

#[derive(Debug, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct InvoicePaymentResponse {
    id: Uuid,
    payer_wallet: String,
    amount_lamports: String,
    tx_sig: String,
    confirmed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendInvoiceRequest {
    wallet_address: String,
    settings_pda: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendInvoiceResponse {
    invoice: InvoiceResponse,
    public_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInvoicePaymentRequest {
    payer_wallet: String,
    amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInvoicePaymentResponse {
    transaction_base64: String,
    recipient: String,
    amount_lamports: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoicePaymentSubmittedRequest {
    payer_wallet: String,
    amount_lamports: String,
    tx_sig: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInvoiceResponse {
    invoice: InvoiceResponse,
    workspace_name: String,
    smart_account_name: String,
    recipient: String,
}

#[derive(Debug, Serialize)]
pub struct InvoiceItemResponse {
    invoice: InvoiceResponse,
}

#[derive(Debug, Serialize)]
pub struct InvoiceListResponse {
    invoices: Vec<InvoiceResponse>,
}

#[derive(Debug, FromRow)]
struct CustomerSnapshot {
    business_name: String,
    billing_email: Option<String>,
    contact_name: Option<String>,
    wallet_address: Option<String>,
}

struct PreparedLineItem {
    description: String,
    quantity_units: i128,
    unit_price_units: i128,
    amount_units: i128,
}

fn parse_date(value: &str, field: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map_err(|_| bad_request(format!("{field} must use YYYY-MM-DD")))
}

fn optional_due_date(value: Option<&str>) -> Result<Option<NaiveDate>, ApiError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| parse_date(value, "dueDate"))
        .transpose()
}

fn prepare_line_items(
    inputs: Vec<InvoiceLineItemInput>,
) -> Result<(Vec<PreparedLineItem>, i128), ApiError> {
    if inputs.is_empty() || inputs.len() > 100 {
        return Err(bad_request(
            "lineItems must contain between 1 and 100 items",
        ));
    }
    let mut total = 0_i128;
    let mut items = Vec::with_capacity(inputs.len());
    for input in inputs {
        let description = input.description.trim();
        if description.is_empty() || description.chars().count() > 500 {
            return Err(bad_request(
                "Each line item needs a description up to 500 characters",
            ));
        }
        let quantity_units = parse_decimal_units(&input.quantity, QUANTITY_SCALE, "quantity")?;
        let unit_price_units = parse_decimal_units(&input.unit_price, MONEY_SCALE, "unitPrice")?;
        if quantity_units == 0 {
            return Err(bad_request("quantity must be greater than zero"));
        }
        let divisor = 10_i128.pow(QUANTITY_SCALE);
        let product = quantity_units
            .checked_mul(unit_price_units)
            .ok_or_else(|| bad_request("Line item amount is too large"))?;
        let amount_units = product
            .checked_add(divisor / 2)
            .ok_or_else(|| bad_request("Line item amount is too large"))?
            / divisor;
        total = total
            .checked_add(amount_units)
            .ok_or_else(|| bad_request("Invoice total is too large"))?;
        items.push(PreparedLineItem {
            description: description.to_owned(),
            quantity_units,
            unit_price_units,
            amount_units,
        });
    }
    Ok((items, total))
}

async fn customer_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    workspace_id: Uuid,
    customer_id: Uuid,
) -> Result<CustomerSnapshot, ApiError> {
    sqlx::query_as::<_, CustomerSnapshot>(
        r#"SELECT business_name, billing_email, contact_name, wallet_address
           FROM customers WHERE id = $1 AND workspace_id = $2 AND archived_at IS NULL"#,
    )
    .bind(customer_id)
    .bind(workspace_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Customer was not found in this workspace"))
}

async fn validate_smart_account(
    transaction: &mut Transaction<'_, Postgres>,
    workspace_id: Uuid,
    smart_account_id: Uuid,
) -> Result<(), ApiError> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
             SELECT 1 FROM smart_accounts sa
             JOIN settings_accounts s ON s.id = sa.settings_account_id
             WHERE sa.id = $1 AND s.workspace_id = $2
           )"#,
    )
    .bind(smart_account_id)
    .bind(workspace_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(internal_error)?;
    if !exists {
        return Err(bad_request(
            "Receiving smart account was not found in this workspace",
        ));
    }
    Ok(())
}

async fn replace_line_items(
    transaction: &mut Transaction<'_, Postgres>,
    invoice_id: Uuid,
    items: &[PreparedLineItem],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM invoice_line_items WHERE invoice_id = $1")
        .bind(invoice_id)
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
    for (position, item) in items.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO invoice_line_items
               (invoice_id, position, description, quantity, unit_price, amount)
               VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6::numeric)"#,
        )
        .bind(invoice_id)
        .bind(position as i32)
        .bind(&item.description)
        .bind(format_units(item.quantity_units, QUANTITY_SCALE))
        .bind(format_units(item.unit_price_units, MONEY_SCALE))
        .bind(format_units(item.amount_units, MONEY_SCALE))
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
    }
    Ok(())
}

async fn load_invoice(
    state: &AppState,
    workspace_id: Uuid,
    id: Uuid,
) -> Result<InvoiceResponse, ApiError> {
    let invoice = sqlx::query_as::<_, InvoiceRow>(
        r#"SELECT id, workspace_id, customer_id, receiving_smart_account_id,
                  invoice_number, status, currency, issue_date, due_date,
                  customer_business_name, customer_billing_email, customer_contact_name,
                  customer_wallet_address, memo, subtotal::text AS subtotal,
                  total::text AS total, amount_paid::text AS amount_paid,
                  created_by_wallet, created_at, updated_at, sent_at, viewed_at
           FROM invoices WHERE id = $1 AND workspace_id = $2"#,
    )
    .bind(id)
    .bind(workspace_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invoice not found"))?;
    let line_items = sqlx::query_as::<_, InvoiceLineItemResponse>(
        r#"SELECT id, position, description, quantity::text AS quantity,
                  unit_price::text AS unit_price, amount::text AS amount
           FROM invoice_line_items WHERE invoice_id = $1 ORDER BY position"#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let payments = sqlx::query_as::<_, InvoicePaymentResponse>(
        r#"SELECT id, payer_wallet, amount_lamports::text AS amount_lamports,
                  tx_sig, confirmed_at
           FROM invoice_payments WHERE invoice_id = $1 ORDER BY confirmed_at DESC"#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(InvoiceResponse {
        invoice,
        line_items,
        payments,
    })
}

pub async fn create_invoice(
    State(state): State<AppState>,
    Json(body): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceItemResponse>), ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let wallet = normalize_wallet_address(&body.wallet_address)?;
    let issue_date = parse_date(&body.issue_date, "issueDate")?;
    let due_date = optional_due_date(body.due_date.as_deref())?;
    if due_date.is_some_and(|date| date < issue_date) {
        return Err(bad_request("dueDate cannot be before issueDate"));
    }
    let memo = body
        .memo
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let (items, total) = prepare_line_items(body.line_items)?;
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    let customer =
        customer_snapshot(&mut transaction, access.workspace_id, body.customer_id).await?;
    validate_smart_account(
        &mut transaction,
        access.workspace_id,
        body.receiving_smart_account_id,
    )
    .await?;
    let invoice_number = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO invoice_sequences (workspace_id, next_value) VALUES ($1, 2)
           ON CONFLICT (workspace_id) DO UPDATE
           SET next_value = invoice_sequences.next_value + 1
           RETURNING next_value - 1"#,
    )
    .bind(access.workspace_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let invoice_id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO invoices (
             workspace_id, customer_id, receiving_smart_account_id, invoice_number,
             issue_date, due_date, customer_business_name, customer_billing_email,
             customer_contact_name, customer_wallet_address, memo, subtotal, total,
             created_by_wallet
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::numeric,$12::numeric,$13)
           RETURNING id"#,
    )
    .bind(access.workspace_id)
    .bind(body.customer_id)
    .bind(body.receiving_smart_account_id)
    .bind(invoice_number)
    .bind(issue_date)
    .bind(due_date)
    .bind(customer.business_name)
    .bind(customer.billing_email)
    .bind(customer.contact_name)
    .bind(customer.wallet_address)
    .bind(memo)
    .bind(format_units(total, MONEY_SCALE))
    .bind(wallet)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    replace_line_items(&mut transaction, invoice_id, &items).await?;
    transaction.commit().await.map_err(internal_error)?;
    let invoice = load_invoice(&state, access.workspace_id, invoice_id).await?;
    Ok((StatusCode::CREATED, Json(InvoiceItemResponse { invoice })))
}

pub async fn list_invoices(
    State(state): State<AppState>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<InvoiceListResponse>, ApiError> {
    let access = customer_access(&state, &query.settings_pda, &query.wallet_address, false).await?;
    let status = query
        .status
        .map(|value| value.trim().to_uppercase())
        .filter(|value| !value.is_empty());
    let (limit, offset) = crate::pagination::bounds(query.limit, query.offset);
    let rows = sqlx::query_as::<_, InvoiceRow>(
        r#"SELECT id, workspace_id, customer_id, receiving_smart_account_id,
                  invoice_number, status, currency, issue_date, due_date,
                  customer_business_name, customer_billing_email, customer_contact_name,
                  customer_wallet_address, memo, subtotal::text AS subtotal,
                  total::text AS total, amount_paid::text AS amount_paid,
                  created_by_wallet, created_at, updated_at, sent_at, viewed_at
           FROM invoices
           WHERE workspace_id = $1 AND ($2::text IS NULL OR status = $2)
           ORDER BY created_at DESC
           LIMIT $3 OFFSET $4"#,
    )
    .bind(access.workspace_id)
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let mut invoices = Vec::with_capacity(rows.len());
    for invoice in rows {
        let line_items = sqlx::query_as::<_, InvoiceLineItemResponse>(
            r#"SELECT id, position, description, quantity::text AS quantity,
                      unit_price::text AS unit_price, amount::text AS amount
               FROM invoice_line_items WHERE invoice_id = $1 ORDER BY position"#,
        )
        .bind(invoice.id)
        .fetch_all(&state.db)
        .await
        .map_err(internal_error)?;
        invoices.push(InvoiceResponse {
            invoice,
            line_items,
            payments: Vec::new(),
        });
    }
    Ok(Json(InvoiceListResponse { invoices }))
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn generate_public_token() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(internal_error)?;
    Ok(hex::encode(bytes))
}

async fn public_invoice_context(
    state: &AppState,
    token: &str,
    mark_viewed: bool,
) -> Result<(Uuid, Uuid, String, String, String), ApiError> {
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bad_request(
            "Invoice link is invalid or no longer available",
        ));
    }
    let hash = token_hash(token);
    let row = sqlx::query_as::<_, (Uuid, Uuid, String, String, String)>(
        r#"SELECT i.id, i.workspace_id, w.name, sa.name, sa.pda
           FROM invoices i
           JOIN workspaces w ON w.id = i.workspace_id
           JOIN smart_accounts sa ON sa.id = i.receiving_smart_account_id
           WHERE i.public_token_hash = $1
             AND i.status NOT IN ('DRAFT', 'VOID')"#,
    )
    .bind(hash)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invoice link is invalid or no longer available"))?;
    if mark_viewed {
        sqlx::query(
            r#"UPDATE invoices SET status = CASE WHEN status='SENT' THEN 'VIEWED' ELSE status END,
               viewed_at = COALESCE(viewed_at, NOW()), updated_at=NOW() WHERE id=$1"#,
        )
        .bind(row.0)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    }
    Ok(row)
}

pub async fn send_invoice(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SendInvoiceRequest>,
) -> Result<Json<SendInvoiceResponse>, ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let token = generate_public_token()?;
    let result = sqlx::query(
        r#"UPDATE invoices SET public_token_hash=$1,
           status=CASE WHEN status IN ('DRAFT','SENT','VIEWED') THEN 'SENT' ELSE status END,
           sent_at=NOW(),
           viewed_at=NULL, updated_at=NOW()
           WHERE id=$2 AND workspace_id=$3 AND status NOT IN ('PAID','VOID') AND total > 0"#,
    )
    .bind(token_hash(&token))
    .bind(id)
    .bind(access.workspace_id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(bad_request("Invoice cannot be sent"));
    }
    Ok(Json(SendInvoiceResponse {
        invoice: load_invoice(&state, access.workspace_id, id).await?,
        public_url: state.config.frontend_url(&format!("/pay/{token}")),
    }))
}

pub async fn get_public_invoice(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<PublicInvoiceResponse>, ApiError> {
    let (id, workspace_id, workspace_name, smart_account_name, recipient) =
        public_invoice_context(&state, &token, true).await?;
    Ok(Json(PublicInvoiceResponse {
        invoice: load_invoice(&state, workspace_id, id).await?,
        workspace_name,
        smart_account_name,
        recipient,
    }))
}

pub async fn build_invoice_payment(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<BuildInvoicePaymentRequest>,
) -> Result<Json<BuildInvoicePaymentResponse>, ApiError> {
    let payer = Pubkey::from_str(body.payer_wallet.trim())
        .map_err(|_| bad_request("payerWallet is invalid"))?;
    let (_, _, _, _, recipient_value) = public_invoice_context(&state, &token, false).await?;
    let recipient = Pubkey::from_str(&recipient_value).map_err(internal_error)?;
    let amount_units = parse_decimal_units(&body.amount, MONEY_SCALE, "amount")?;
    let amount_lamports =
        u64::try_from(amount_units).map_err(|_| bad_request("amount is too large"))?;
    if amount_lamports == 0 {
        return Err(bad_request("amount must be greater than zero"));
    }
    let remaining = sqlx::query_scalar::<_, String>(
        r#"SELECT ((total - amount_paid) * 1000000000)::numeric(20,0)::text
           FROM invoices WHERE public_token_hash=$1"#,
    )
    .bind(token_hash(token.trim()))
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?
    .parse::<u64>()
    .map_err(internal_error)?;
    if amount_lamports > remaining {
        return Err(bad_request("Payment exceeds the remaining invoice balance"));
    }
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    let instruction = system_instruction::transfer(&payer, &recipient, amount_lamports);
    let transaction_base64 =
        build_unsigned_transaction_base64(payer, blockhash, instruction).map_err(internal_error)?;
    Ok(Json(BuildInvoicePaymentResponse {
        transaction_base64,
        recipient: recipient.to_string(),
        amount_lamports: amount_lamports.to_string(),
    }))
}

pub async fn invoice_payment_submitted(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<InvoicePaymentSubmittedRequest>,
) -> Result<Json<PublicInvoiceResponse>, ApiError> {
    let payer = Pubkey::from_str(body.payer_wallet.trim())
        .map_err(|_| bad_request("payerWallet is invalid"))?;
    let signature =
        Signature::from_str(body.tx_sig.trim()).map_err(|_| bad_request("txSig is invalid"))?;
    let amount_lamports = body
        .amount_lamports
        .trim()
        .parse::<u64>()
        .map_err(|_| bad_request("amountLamports is invalid"))?;
    let (invoice_id, workspace_id, workspace_name, smart_account_name, recipient_value) =
        public_invoice_context(&state, &token, false).await?;
    let recipient = Pubkey::from_str(&recipient_value).map_err(internal_error)?;
    verify_confirmed_sol_transfer(
        state.rpc.as_ref(),
        &signature,
        &payer,
        &recipient,
        amount_lamports,
    )
    .await
    .map_err(bad_request)?;

    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    let current = sqlx::query_as::<_, (String, String)>(
        "SELECT total::text, amount_paid::text FROM invoices WHERE id=$1 FOR UPDATE",
    )
    .bind(invoice_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let total = parse_decimal_units(&current.0, MONEY_SCALE, "total")?;
    let paid = parse_decimal_units(&current.1, MONEY_SCALE, "amountPaid")?;
    let amount = i128::from(amount_lamports);
    if paid + amount > total {
        return Err(bad_request("Payment exceeds the remaining invoice balance"));
    }
    let inserted = sqlx::query(
        r#"INSERT INTO invoice_payments (invoice_id, payer_wallet, amount_lamports, tx_sig)
           VALUES ($1,$2,$3::numeric,$4) ON CONFLICT (tx_sig) DO NOTHING"#,
    )
    .bind(invoice_id)
    .bind(payer.to_string())
    .bind(amount_lamports.to_string())
    .bind(signature.to_string())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if inserted.rows_affected() == 0 {
        return Err(bad_request("This transaction was already recorded"));
    }
    let new_paid = paid + amount;
    let status = if new_paid == total {
        "PAID"
    } else {
        "PARTIALLY_PAID"
    };
    sqlx::query(
        r#"UPDATE invoices SET amount_paid=$1::numeric, status=$2,
           updated_at=NOW() WHERE id=$3"#,
    )
    .bind(format_units(new_paid, MONEY_SCALE))
    .bind(status)
    .bind(invoice_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(PublicInvoiceResponse {
        invoice: load_invoice(&state, workspace_id, invoice_id).await?,
        workspace_name,
        smart_account_name,
        recipient: recipient.to_string(),
    }))
}

pub async fn get_invoice(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<InvoiceAccessQuery>,
) -> Result<Json<InvoiceItemResponse>, ApiError> {
    let access = customer_access(&state, &query.settings_pda, &query.wallet_address, false).await?;
    Ok(Json(InvoiceItemResponse {
        invoice: load_invoice(&state, access.workspace_id, id).await?,
    }))
}

pub async fn update_invoice(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateInvoiceRequest>,
) -> Result<Json<InvoiceItemResponse>, ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let mut transaction = state.db.begin().await.map_err(internal_error)?;
    let current = sqlx::query_as::<_, (Uuid, Uuid, NaiveDate, Option<NaiveDate>, Option<String>, String)>(
        "SELECT customer_id, receiving_smart_account_id, issue_date, due_date, memo, status FROM invoices WHERE id=$1 AND workspace_id=$2 FOR UPDATE"
    ).bind(id).bind(access.workspace_id).fetch_optional(&mut *transaction).await.map_err(internal_error)?
      .ok_or_else(|| bad_request("Invoice not found"))?;
    if current.5 != "DRAFT" {
        return Err(bad_request("Only draft invoices can be edited"));
    }
    let customer_id = body.customer_id.unwrap_or(current.0);
    let smart_account_id = body.receiving_smart_account_id.unwrap_or(current.1);
    let issue_date = body
        .issue_date
        .as_deref()
        .map(|v| parse_date(v, "issueDate"))
        .transpose()?
        .unwrap_or(current.2);
    let due_date = if body.due_date.is_some() {
        optional_due_date(body.due_date.as_deref())?
    } else {
        current.3
    };
    if due_date.is_some_and(|date| date < issue_date) {
        return Err(bad_request("dueDate cannot be before issueDate"));
    }
    let memo = body
        .memo
        .map(|v| v.trim().to_owned())
        .map(|v| if v.is_empty() { None } else { Some(v) })
        .unwrap_or(current.4);
    let customer = customer_snapshot(&mut transaction, access.workspace_id, customer_id).await?;
    validate_smart_account(&mut transaction, access.workspace_id, smart_account_id).await?;
    let prepared = body.line_items.map(prepare_line_items).transpose()?;
    let total = prepared
        .as_ref()
        .map(|(_, total)| format_units(*total, MONEY_SCALE));
    sqlx::query(
        r#"UPDATE invoices SET customer_id=$1, receiving_smart_account_id=$2,
           issue_date=$3, due_date=$4, customer_business_name=$5, customer_billing_email=$6,
           customer_contact_name=$7, customer_wallet_address=$8, memo=$9,
           subtotal=COALESCE($10::numeric, subtotal), total=COALESCE($10::numeric, total), updated_at=NOW()
           WHERE id=$11"#,
    ).bind(customer_id).bind(smart_account_id).bind(issue_date).bind(due_date)
      .bind(customer.business_name).bind(customer.billing_email).bind(customer.contact_name)
      .bind(customer.wallet_address).bind(memo).bind(total).bind(id)
      .execute(&mut *transaction).await.map_err(internal_error)?;
    if let Some((items, _)) = prepared {
        replace_line_items(&mut transaction, id, &items).await?;
    }
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(InvoiceItemResponse {
        invoice: load_invoice(&state, access.workspace_id, id).await?,
    }))
}

pub async fn delete_invoice(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<InvoiceAccessQuery>,
) -> Result<StatusCode, ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let result =
        sqlx::query("DELETE FROM invoices WHERE id=$1 AND workspace_id=$2 AND status='DRAFT'")
            .bind(id)
            .bind(access.workspace_id)
            .execute(&state.db)
            .await
            .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(bad_request("Draft invoice not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_math_is_exact() {
        let (items, total) = prepare_line_items(vec![InvoiceLineItemInput {
            description: "Service".into(),
            quantity: "3".into(),
            unit_price: "0.1".into(),
        }])
        .unwrap();
        assert_eq!(
            format_units(items[0].amount_units, MONEY_SCALE),
            "0.300000000"
        );
        assert_eq!(format_units(total, MONEY_SCALE), "0.300000000");
    }

    #[test]
    fn rejects_excess_precision() {
        assert!(parse_decimal_units("1.0000000001", MONEY_SCALE, "amount").is_err());
    }

    #[test]
    fn public_tokens_have_256_bits_of_random_data() {
        let first = generate_public_token().unwrap();
        let second = generate_public_token().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert_eq!(token_hash(&first).len(), 64);
    }
}
