use std::str::FromStr;

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Response, header},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState,
    access::{SettingsAccess, require_settings_access, require_settings_member_by_id},
    bad_request, internal_error,
    money::parse_decimal_units,
};

use super::{auth::normalize_wallet_address, extension_auth::extension_token_hash};

const MAX_RECEIPT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, FromRow)]
struct ReceiptRow {
    id: Uuid,
    settings_account_id: Uuid,
    smart_account_id: Option<Uuid>,
    transaction_record_id: Option<Uuid>,
    uploaded_by_wallet: String,
    file_pathname: String,
    file_name: Option<String>,
    file_mime_type: Option<String>,
    file_size_bytes: Option<i32>,
    vendor: Option<String>,
    amount: Option<String>,
    currency: Option<String>,
    due_date: Option<DateTime<Utc>>,
    invoice_number: Option<String>,
    category: Option<String>,
    payment_recipient: Option<String>,
    confidence: Option<f64>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    workspace_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicReceipt {
    id: Uuid,
    settings_account_id: Uuid,
    smart_account_id: Option<Uuid>,
    transaction_record_id: Option<Uuid>,
    uploaded_by_wallet: String,
    file_url: String,
    file_name: Option<String>,
    file_mime_type: Option<String>,
    file_size_bytes: Option<i32>,
    vendor: Option<String>,
    amount: Option<String>,
    currency: Option<String>,
    due_date: Option<DateTime<Utc>>,
    invoice_number: Option<String>,
    category: Option<String>,
    payment_recipient: Option<String>,
    confidence: Option<f64>,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReceiptResponse {
    receipt: PublicReceipt,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReceiptQuery {
    wallet_address: String,
    settings_pda: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReceiptRequest {
    wallet_address: String,
    smart_account_id: Option<Uuid>,
    vendor: Option<String>,
    amount: Option<serde_json::Value>,
    currency: Option<String>,
    due_date: Option<String>,
    invoice_number: Option<String>,
    category: Option<String>,
    payment_recipient: Option<String>,
    confidence: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReceiptRequest {
    wallet_address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReceiptResponse {
    ok: bool,
    receipt_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptFileQuery {
    wallet_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptPaymentRequest {
    wallet_address: String,
    transaction_record_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeProcessedReceipt {
    source_mime_type: String,
    extraction_mime_type: String,
    extraction: ExtractedReceipt,
    file: NodeFile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractedReceipt {
    vendor: Option<String>,
    amount: Option<serde_json::Value>,
    currency: Option<String>,
    due_date: Option<String>,
    invoice_number: Option<String>,
    category: Option<String>,
    payment_recipient: Option<String>,
    confidence: Option<f64>,
    raw_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeFile {
    url: String,
    pathname: String,
    name: String,
    mime_type: String,
    size_bytes: usize,
}

fn internal_token(state: &AppState) -> Result<&str, ApiError> {
    state
        .config
        .internal_service_token
        .as_deref()
        .ok_or_else(|| internal_error("INTERNAL_SERVICE_TOKEN is required"))
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| bad_request("Extension session token is required"))?;
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && !token.contains(char::is_whitespace))
        .ok_or_else(|| bad_request("Extension session token is required"))
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn validate_file(mime_type: &str, body: &Bytes) -> Result<(), ApiError> {
    if !matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/webp" | "application/pdf" | "text/plain"
    ) {
        return Err(bad_request(
            "Upload an image, PDF, or selected invoice text.",
        ));
    }
    if body.is_empty() {
        return Err(bad_request("Receipt file is empty."));
    }
    if body.len() > MAX_RECEIPT_BYTES {
        return Err(bad_request("Receipt file must be 20MB or smaller."));
    }
    Ok(())
}

fn amount_string(value: Option<&serde_json::Value>) -> Result<Option<String>, ApiError> {
    let Some(value) = value else { return Ok(None) };
    let value = match value {
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.trim().to_owned(),
        serde_json::Value::Null => return Ok(None),
        _ => return Err(bad_request("amount is invalid")),
    };
    parse_decimal_units(&value, 9, "amount")?;
    Ok(Some(value))
}

fn due_date(value: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| bad_request("dueDate must use YYYY-MM-DD"))?;
    Ok(Some(DateTime::from_naive_utc_and_offset(
        date.and_hms_opt(0, 0, 0).expect("midnight is valid"),
        Utc,
    )))
}

fn validate_recipient(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    Pubkey::from_str(value)
        .map(|key| Some(key.to_string()))
        .map_err(|_| bad_request("paymentRecipient is invalid"))
}

fn extracted_recipient(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| Pubkey::from_str(value).ok())
        .map(|key| key.to_string())
}

fn public_receipt(row: ReceiptRow) -> PublicReceipt {
    PublicReceipt {
        file_url: format!("/receipts/{}/file", row.id),
        id: row.id,
        settings_account_id: row.settings_account_id,
        smart_account_id: row.smart_account_id,
        transaction_record_id: row.transaction_record_id,
        uploaded_by_wallet: row.uploaded_by_wallet,
        file_name: row.file_name,
        file_mime_type: row.file_mime_type,
        file_size_bytes: row.file_size_bytes,
        vendor: row.vendor,
        amount: row.amount,
        currency: row.currency,
        due_date: row.due_date,
        invoice_number: row.invoice_number,
        category: row.category,
        payment_recipient: row.payment_recipient,
        confidence: row.confidence,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

const RECEIPT_SELECT: &str = r#"
    SELECT receipt.id, receipt.settings_account_id, receipt.smart_account_id,
           receipt.transaction_record_id, receipt.uploaded_by_wallet,
           receipt.file_pathname, receipt.file_name,
           receipt.file_mime_type, receipt.file_size_bytes, receipt.vendor,
           receipt.amount::text AS amount, receipt.currency, receipt.due_date,
           receipt.invoice_number, receipt.category, receipt.payment_recipient,
           receipt.confidence, receipt.status, receipt.created_at,
           receipt.updated_at, settings.workspace_id
    FROM receipts AS receipt
    JOIN settings_accounts AS settings ON settings.id = receipt.settings_account_id
    WHERE receipt.id = $1
"#;

async fn load_receipt(state: &AppState, id: Uuid) -> Result<ReceiptRow, ApiError> {
    sqlx::query_as::<_, ReceiptRow>(RECEIPT_SELECT)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| bad_request("Receipt not found"))
}

pub(crate) async fn load_receipts_for_settings(
    state: &AppState,
    settings_id: Uuid,
) -> Result<Vec<PublicReceipt>, ApiError> {
    let rows = sqlx::query_as::<_, ReceiptRow>(
        r#"
        SELECT receipt.id, receipt.settings_account_id, receipt.smart_account_id,
               receipt.transaction_record_id, receipt.uploaded_by_wallet,
               receipt.file_pathname, receipt.file_name,
               receipt.file_mime_type, receipt.file_size_bytes, receipt.vendor,
               receipt.amount::text AS amount, receipt.currency, receipt.due_date,
               receipt.invoice_number, receipt.category, receipt.payment_recipient,
               receipt.confidence, receipt.status, receipt.created_at,
               receipt.updated_at, settings.workspace_id
        FROM receipts AS receipt
        JOIN settings_accounts AS settings ON settings.id = receipt.settings_account_id
        WHERE receipt.settings_account_id = $1
        ORDER BY receipt.created_at DESC
        LIMIT 100
        "#,
    )
    .bind(settings_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;

    Ok(rows.into_iter().map(public_receipt).collect())
}

pub(crate) async fn link_receipt_to_transaction(
    state: &AppState,
    receipt_id: Uuid,
    settings_id: Uuid,
    smart_account_id: Uuid,
    transaction_id: Uuid,
) -> Result<(), ApiError> {
    let result = sqlx::query(
        r#"
        UPDATE receipts
        SET transaction_record_id = $4,
            smart_account_id = $3,
            status = 'PAYMENT_PREPARED',
            updated_at = NOW()
        WHERE id = $1
          AND settings_account_id = $2
          AND status <> 'PAID'
          AND (transaction_record_id IS NULL OR transaction_record_id = $4)
        "#,
    )
    .bind(receipt_id)
    .bind(settings_id)
    .bind(smart_account_id)
    .bind(transaction_id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;

    if result.rows_affected() != 1 {
        return Err(bad_request(
            "Receipt is unavailable, already paid, or linked to another transaction",
        ));
    }
    Ok(())
}

pub(crate) async fn mark_transaction_receipt_paid(
    state: &AppState,
    transaction_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE receipts SET status='PAID', updated_at=NOW() WHERE transaction_record_id=$1",
    )
    .bind(transaction_id)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(())
}

async fn settings_access(
    state: &AppState,
    settings_pda: &str,
    wallet: &str,
) -> Result<SettingsAccess, ApiError> {
    require_settings_access(state, settings_pda, wallet, None, false).await
}

async fn ensure_receipt_access(
    state: &AppState,
    receipt: &ReceiptRow,
    wallet: &str,
) -> Result<(), ApiError> {
    require_settings_member_by_id(state, receipt.settings_account_id, wallet).await
}

async fn process_file(
    state: &AppState,
    settings_id: Uuid,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<NodeProcessedReceipt, ApiError> {
    let mime_type = header_string(headers, "content-type")
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let mime_type = mime_type.split(';').next().unwrap_or("").trim();
    validate_file(mime_type, body)?;
    let file_name = header_string(headers, "x-file-name").unwrap_or_else(|| "receipt".to_owned());
    let response = state
        .http
        .post(state.config.node_url("/internal/receipts/process"))
        .bearer_auth(internal_token(state)?)
        .header("content-type", mime_type)
        .header("x-file-name", file_name)
        .header("x-hover-settings-account-id", settings_id.to_string())
        .body(body.clone())
        .send()
        .await
        .map_err(internal_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(internal_error(format!(
            "Receipt processor failed with {status}: {message}"
        )));
    }
    response.json().await.map_err(internal_error)
}

async fn delete_internal_file(state: &AppState, pathname: &str) -> Result<(), ApiError> {
    let response = state
        .http
        .post(state.config.node_url("/internal/receipts/file/delete"))
        .bearer_auth(internal_token(state)?)
        .json(&serde_json::json!({ "pathname": pathname }))
        .send()
        .await
        .map_err(internal_error)?;
    if !response.status().is_success() {
        return Err(internal_error(format!(
            "Receipt file deletion failed with {}",
            response.status()
        )));
    }
    Ok(())
}

async fn insert_processed_receipt(
    state: &AppState,
    settings: &SettingsAccess,
    wallet: &str,
    smart_account_id: Option<Uuid>,
    source_url: Option<String>,
    source_title: Option<String>,
    processed: NodeProcessedReceipt,
) -> Result<ReceiptRow, ApiError> {
    if let Some(smart_account_id) = smart_account_id {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM smart_accounts WHERE id = $1 AND settings_account_id = $2)",
        )
        .bind(smart_account_id)
        .bind(settings.settings_account_id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;
        if !valid {
            let _ = delete_internal_file(state, &processed.file.pathname).await;
            return Err(bad_request(
                "Smart account does not belong to this settings account",
            ));
        }
    }
    let amount = amount_string(processed.extraction.amount.as_ref())?;
    let due = due_date(processed.extraction.due_date.as_deref())?;
    // OCR output is untrusted and optional. Keep an invalid inferred address out
    // of the indexed fields so the user can review and enter it explicitly.
    let recipient = extracted_recipient(processed.extraction.payment_recipient.as_deref());
    let status = if processed.extraction_mime_type == "text/plain"
        || processed.source_mime_type.starts_with("image/")
    {
        "EXTRACTED"
    } else {
        "UPLOADED"
    };
    let extracted_json = serde_json::json!({
        "vendor": processed.extraction.vendor,
        "amount": processed.extraction.amount,
        "currency": processed.extraction.currency,
        "dueDate": processed.extraction.due_date,
        "invoiceNumber": processed.extraction.invoice_number,
        "category": processed.extraction.category,
        "paymentRecipient": processed.extraction.payment_recipient,
        "confidence": processed.extraction.confidence,
        "rawText": processed.extraction.raw_text,
        "source": { "url": source_url, "title": source_title }
    });
    let result = sqlx::query_as::<_, ReceiptRow>(
        r#"
        INSERT INTO receipts (
            settings_account_id, smart_account_id, uploaded_by_wallet,
            file_url, file_pathname, file_name, file_mime_type, file_size_bytes,
            vendor, amount, currency, due_date, invoice_number, category,
            payment_recipient, confidence, status, raw_ocr_text, extracted_json
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::numeric,$11,$12,$13,$14,$15,$16,$17,$18,$19)
        RETURNING id, settings_account_id, smart_account_id, transaction_record_id,
                  uploaded_by_wallet, file_pathname, file_name,
                  file_mime_type, file_size_bytes, vendor, amount::text AS amount,
                  currency, due_date, invoice_number, category, payment_recipient,
                  confidence, status, created_at, updated_at, $20 AS workspace_id
        "#,
    )
    .bind(settings.settings_account_id)
    .bind(smart_account_id)
    .bind(wallet)
    .bind(&processed.file.url)
    .bind(&processed.file.pathname)
    .bind(&processed.file.name)
    .bind(&processed.file.mime_type)
    .bind(i32::try_from(processed.file.size_bytes).map_err(internal_error)?)
    .bind(processed.extraction.vendor.as_deref())
    .bind(amount.as_deref())
    .bind(processed.extraction.currency.as_deref())
    .bind(due)
    .bind(processed.extraction.invoice_number.as_deref())
    .bind(processed.extraction.category.as_deref())
    .bind(recipient.as_deref())
    .bind(processed.extraction.confidence)
    .bind(status)
    .bind(processed.extraction.raw_text.as_deref())
    .bind(extracted_json)
    .bind(settings.workspace_id)
    .fetch_one(&state.db)
    .await;
    match result {
        Ok(receipt) => {
            sqlx::query(
                r#"INSERT INTO activity_logs (workspace_id, settings_account_id, smart_account_id,
                   activity_type, title, metadata)
                   VALUES ($1,$2,$3,'RECEIPT_SCANNED','Receipt uploaded',$4)"#,
            )
            .bind(settings.workspace_id)
            .bind(settings.settings_account_id)
            .bind(smart_account_id)
            .bind(serde_json::json!({ "receiptId": receipt.id, "fileName": receipt.file_name }))
            .execute(&state.db)
            .await
            .map_err(internal_error)?;
            Ok(receipt)
        }
        Err(error) => {
            let _ = delete_internal_file(state, &processed.file.pathname).await;
            Err(internal_error(error))
        }
    }
}

pub async fn scan_receipt(
    State(state): State<AppState>,
    Query(query): Query<ScanReceiptQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ReceiptResponse>, ApiError> {
    let wallet = normalize_wallet_address(&query.wallet_address)?;
    let settings_pda = Pubkey::from_str(query.settings_pda.trim())
        .map_err(|_| bad_request("settingsPda is invalid"))?
        .to_string();
    let settings = settings_access(&state, &settings_pda, &wallet).await?;
    let processed = process_file(&state, settings.settings_account_id, &headers, &body).await?;
    let receipt =
        insert_processed_receipt(&state, &settings, &wallet, None, None, None, processed).await?;
    Ok(Json(ReceiptResponse {
        receipt: public_receipt(receipt),
    }))
}

pub async fn extension_scan_receipt(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ReceiptResponse>, ApiError> {
    let token_hash = extension_token_hash(&state, bearer_token(&headers)?)?;
    let session = sqlx::query_as::<_, SettingsAccess>(
        r#"SELECT settings.id AS settings_account_id, settings.workspace_id,
                  settings.settings_authority,
                  COALESCE(signer.permissions_mask, 0) AS permissions_mask
           FROM extension_sessions AS session
           JOIN settings_accounts AS settings ON settings.id = session.settings_account_id
           LEFT JOIN settings_signers AS signer
             ON signer.settings_account_id = settings.id
            AND signer.wallet_address = session.wallet_address
           WHERE session.token_hash = $1 AND session.revoked_at IS NULL
             AND session.expires_at > NOW()"#,
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Extension session is not active"))?;
    let wallet = sqlx::query_scalar::<_, String>(
        "UPDATE extension_sessions SET last_used_at = NOW(), updated_at = NOW() WHERE token_hash = $1 RETURNING wallet_address",
    )
    .bind(token_hash)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    let smart_account_id = header_string(&headers, "x-hover-smart-account-id")
        .map(|value| Uuid::parse_str(&value).map_err(|_| bad_request("smartAccountId is invalid")))
        .transpose()?;
    let source_url = header_string(&headers, "x-hover-source-url");
    let source_title = header_string(&headers, "x-hover-source-title");
    let processed = process_file(&state, session.settings_account_id, &headers, &body).await?;
    let receipt = insert_processed_receipt(
        &state,
        &session,
        &wallet,
        smart_account_id,
        source_url,
        source_title,
        processed,
    )
    .await?;
    Ok(Json(ReceiptResponse {
        receipt: public_receipt(receipt),
    }))
}

fn ensure_editable(receipt: &ReceiptRow) -> Result<(), ApiError> {
    if receipt.transaction_record_id.is_some()
        || matches!(receipt.status.as_str(), "PAYMENT_PREPARED" | "PAID")
    {
        return Err(bad_request(
            "Receipts with prepared or completed payments cannot be edited.",
        ));
    }
    Ok(())
}

pub async fn update_receipt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateReceiptRequest>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    let wallet = normalize_wallet_address(&payload.wallet_address)?;
    let receipt = load_receipt(&state, id).await?;
    ensure_receipt_access(&state, &receipt, &wallet).await?;
    ensure_editable(&receipt)?;
    if let Some(smart_account_id) = payload.smart_account_id {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM smart_accounts WHERE id=$1 AND settings_account_id=$2)",
        )
        .bind(smart_account_id)
        .bind(receipt.settings_account_id)
        .fetch_one(&state.db)
        .await
        .map_err(internal_error)?;
        if !valid {
            return Err(bad_request(
                "Smart account does not belong to this settings account",
            ));
        }
    }
    if payload
        .confidence
        .is_some_and(|value| !(0.0..=1.0).contains(&value))
    {
        return Err(bad_request("confidence must be between 0 and 1"));
    }
    let amount = amount_string(payload.amount.as_ref())?;
    let due = due_date(payload.due_date.as_deref())?;
    let recipient = validate_recipient(payload.payment_recipient.as_deref())?;
    sqlx::query(
        r#"UPDATE receipts SET
           smart_account_id=COALESCE($2,smart_account_id), vendor=COALESCE($3,vendor),
           amount=COALESCE($4::numeric,amount), currency=COALESCE($5,currency),
           due_date=COALESCE($6,due_date), invoice_number=COALESCE($7,invoice_number),
           category=COALESCE($8,category), payment_recipient=COALESCE($9,payment_recipient),
           confidence=COALESCE($10,confidence), status='REVIEWED', updated_at=NOW()
           WHERE id=$1"#,
    )
    .bind(id)
    .bind(payload.smart_account_id)
    .bind(payload.vendor.as_deref())
    .bind(amount.as_deref())
    .bind(payload.currency.as_deref())
    .bind(due)
    .bind(payload.invoice_number.as_deref())
    .bind(payload.category.as_deref())
    .bind(recipient.as_deref())
    .bind(payload.confidence)
    .execute(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(ReceiptResponse {
        receipt: public_receipt(load_receipt(&state, id).await?),
    }))
}

pub async fn delete_receipt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<DeleteReceiptRequest>,
) -> Result<Json<DeleteReceiptResponse>, ApiError> {
    let wallet = normalize_wallet_address(&payload.wallet_address)?;
    let receipt = load_receipt(&state, id).await?;
    ensure_receipt_access(&state, &receipt, &wallet).await?;
    if receipt.transaction_record_id.is_some()
        || matches!(receipt.status.as_str(), "PAYMENT_PREPARED" | "PAID")
    {
        return Err(bad_request(
            "Receipts with prepared or completed payments cannot be deleted.",
        ));
    }
    delete_internal_file(&state, &receipt.file_pathname).await?;
    sqlx::query("DELETE FROM receipts WHERE id=$1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    sqlx::query(r#"INSERT INTO activity_logs (workspace_id,settings_account_id,smart_account_id,
        activity_type,title,metadata) VALUES ($1,$2,$3,'RECEIPT_ARCHIVED','Receipt deleted',$4)"#)
        .bind(receipt.workspace_id).bind(receipt.settings_account_id).bind(receipt.smart_account_id)
        .bind(serde_json::json!({"receiptId":id,"fileName":receipt.file_name,"filePathname":receipt.file_pathname}))
        .execute(&state.db).await.map_err(internal_error)?;
    Ok(Json(DeleteReceiptResponse {
        ok: true,
        receipt_id: id,
    }))
}

pub async fn receipt_file(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ReceiptFileQuery>,
) -> Result<Response<Body>, ApiError> {
    let wallet = normalize_wallet_address(&query.wallet_address)?;
    let receipt = load_receipt(&state, id).await?;
    ensure_receipt_access(&state, &receipt, &wallet).await?;
    let response = state
        .http
        .get(state.config.node_url("/internal/receipts/file"))
        .bearer_auth(internal_token(&state)?)
        .query(&[("pathname", &receipt.file_pathname)])
        .send()
        .await
        .map_err(internal_error)?;
    if !response.status().is_success() {
        return Err(bad_request("Receipt file is not available"));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let bytes = response.bytes().await.map_err(internal_error)?;
    let filename = receipt
        .file_name
        .unwrap_or_else(|| "receipt".to_owned())
        .replace(['"', '\r', '\n'], "");
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, max-age=60")
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{filename}\""),
        )
        .body(Body::from(bytes))
        .map_err(internal_error)
}

pub async fn receipt_payment_submitted(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReceiptPaymentRequest>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    let wallet = normalize_wallet_address(&payload.wallet_address)?;
    let receipt = load_receipt(&state, id).await?;
    ensure_receipt_access(&state, &receipt, &wallet).await?;
    if receipt.status == "PAID" {
        return Err(bad_request("Paid receipts cannot be relinked."));
    }
    if receipt
        .transaction_record_id
        .is_some_and(|existing| existing != payload.transaction_record_id)
    {
        return Err(bad_request(
            "Receipt is already linked to another transaction",
        ));
    }
    let smart_account_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT smart_account_id FROM transaction_records WHERE id=$1 AND settings_account_id=$2",
    )
    .bind(payload.transaction_record_id)
    .bind(receipt.settings_account_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Transaction does not belong to this settings account"))?;
    link_receipt_to_transaction(
        &state,
        id,
        receipt.settings_account_id,
        smart_account_id,
        payload.transaction_record_id,
    )
    .await?;
    Ok(Json(ReceiptResponse {
        receipt: public_receipt(load_receipt(&state, id).await?),
    }))
}

#[cfg(test)]
mod tests {
    use super::{amount_string, due_date, extracted_recipient, validate_recipient};
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn validates_receipt_amounts() {
        assert!(matches!(
            amount_string(Some(&serde_json::json!("3.65"))),
            Ok(Some(value)) if value == "3.65"
        ));
        assert!(amount_string(Some(&serde_json::json!(-1))).is_err());
        assert!(amount_string(Some(&serde_json::json!("not-money"))).is_err());
    }

    #[test]
    fn validates_iso_due_dates() {
        assert!(due_date(Some("2026-09-29")).is_ok());
        assert!(due_date(Some("29/09/2026")).is_err());
    }

    #[test]
    fn validates_solana_payment_recipients() {
        assert!(validate_recipient(Some("11111111111111111111111111111111")).is_ok());
        assert!(validate_recipient(Some("not-a-wallet")).is_err());
    }

    #[test]
    fn ignores_invalid_ocr_payment_recipient() {
        assert_eq!(extracted_recipient(Some("Helius billing account")), None);
    }

    #[test]
    fn keeps_valid_ocr_payment_recipient() {
        let recipient = Pubkey::new_unique().to_string();
        assert_eq!(extracted_recipient(Some(&recipient)), Some(recipient));
    }
}
