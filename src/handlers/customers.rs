use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    ApiError, AppState,
    access::{SettingsAccess, require_settings_access},
    bad_request, internal_error,
};

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerResponse {
    id: Uuid,
    workspace_id: Uuid,
    business_name: String,
    billing_email: Option<String>,
    contact_name: Option<String>,
    wallet_address: Option<String>,
    notes: Option<String>,
    archived_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CustomerListResponse {
    customers: Vec<CustomerResponse>,
}

#[derive(Debug, Serialize)]
pub struct CustomerItemResponse {
    customer: CustomerResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCustomersQuery {
    wallet_address: String,
    settings_pda: String,
    #[serde(default)]
    include_archived: bool,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCustomerRequest {
    wallet_address: String,
    settings_pda: String,
    business_name: String,
    billing_email: Option<String>,
    contact_name: Option<String>,
    customer_wallet_address: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCustomerRequest {
    wallet_address: String,
    settings_pda: String,
    business_name: Option<String>,
    billing_email: Option<String>,
    contact_name: Option<String>,
    customer_wallet_address: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCustomerRequest {
    wallet_address: String,
    settings_pda: String,
}

fn normalize_required_text(value: &str, field: &str, max_len: usize) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request(format!("{field} is required")));
    }
    if value.chars().count() > max_len {
        return Err(bad_request(format!("{field} is too long")));
    }
    Ok(value.to_owned())
}

fn normalize_optional_text(
    value: Option<String>,
    field: &str,
    max_len: usize,
) -> Result<Option<String>, ApiError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > max_len {
                return Err(bad_request(format!("{field} is too long")));
            }
            Ok(Some(value.to_owned()))
        })
        .transpose()
        .map(Option::flatten)
}

fn normalize_email(value: Option<String>) -> Result<Option<String>, ApiError> {
    let value = normalize_optional_text(value, "billingEmail", 254)?;
    if let Some(email) = value.as_deref()
        && (!email.contains('@') || email.starts_with('@') || email.ends_with('@'))
    {
        return Err(bad_request("billingEmail is invalid"));
    }
    Ok(value.map(|email| email.to_lowercase()))
}

fn normalize_customer_wallet(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(|value| value.trim().to_owned()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    Pubkey::from_str(&value)
        .map(|wallet| Some(wallet.to_string()))
        .map_err(|_| bad_request("customerWalletAddress is invalid"))
}

pub(super) async fn customer_access(
    state: &AppState,
    settings_pda: &str,
    wallet_address: &str,
    authority_required: bool,
) -> Result<SettingsAccess, ApiError> {
    require_settings_access(
        state,
        settings_pda,
        wallet_address,
        None,
        authority_required,
    )
    .await
}

pub async fn list_customers(
    State(state): State<AppState>,
    Query(query): Query<ListCustomersQuery>,
) -> Result<Json<CustomerListResponse>, ApiError> {
    let access = customer_access(&state, &query.settings_pda, &query.wallet_address, false).await?;
    let (limit, offset) = crate::pagination::bounds(query.limit, query.offset);
    let customers = sqlx::query_as::<_, CustomerResponse>(
        r#"
        SELECT id, workspace_id, business_name, billing_email, contact_name,
               wallet_address, notes, archived_at, created_at, updated_at
        FROM customers
        WHERE workspace_id = $1
          AND ($2 OR archived_at IS NULL)
        ORDER BY archived_at NULLS FIRST, lower(business_name), created_at
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(access.workspace_id)
    .bind(query.include_archived)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    Ok(Json(CustomerListResponse { customers }))
}

pub async fn create_customer(
    State(state): State<AppState>,
    Json(body): Json<CreateCustomerRequest>,
) -> Result<(StatusCode, Json<CustomerItemResponse>), ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let business_name = normalize_required_text(&body.business_name, "businessName", 160)?;
    let billing_email = normalize_email(body.billing_email)?;
    let contact_name = normalize_optional_text(body.contact_name, "contactName", 160)?;
    let customer_wallet_address = normalize_customer_wallet(body.customer_wallet_address)?;
    let notes = normalize_optional_text(body.notes, "notes", 4000)?;
    let customer = sqlx::query_as::<_, CustomerResponse>(
        r#"
        INSERT INTO customers (
            workspace_id, business_name, billing_email, contact_name,
            wallet_address, notes
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, workspace_id, business_name, billing_email, contact_name,
                  wallet_address, notes, archived_at, created_at, updated_at
        "#,
    )
    .bind(access.workspace_id)
    .bind(business_name)
    .bind(billing_email)
    .bind(contact_name)
    .bind(customer_wallet_address)
    .bind(notes)
    .fetch_one(&state.db)
    .await
    .map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(CustomerItemResponse { customer })))
}

pub async fn update_customer(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCustomerRequest>,
) -> Result<Json<CustomerItemResponse>, ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let billing_email_present = body.billing_email.is_some();
    let contact_name_present = body.contact_name.is_some();
    let customer_wallet_present = body.customer_wallet_address.is_some();
    let notes_present = body.notes.is_some();
    let business_name = body
        .business_name
        .as_deref()
        .map(|value| normalize_required_text(value, "businessName", 160))
        .transpose()?;
    let billing_email = normalize_email(body.billing_email)?;
    let contact_name = normalize_optional_text(body.contact_name, "contactName", 160)?;
    let customer_wallet_address = normalize_customer_wallet(body.customer_wallet_address)?;
    let notes = normalize_optional_text(body.notes, "notes", 4000)?;
    let customer = sqlx::query_as::<_, CustomerResponse>(
        r#"
        UPDATE customers
        SET business_name = COALESCE($3, business_name),
            billing_email = CASE WHEN $4 THEN $5 ELSE billing_email END,
            contact_name = CASE WHEN $6 THEN $7 ELSE contact_name END,
            wallet_address = CASE WHEN $8 THEN $9 ELSE wallet_address END,
            notes = CASE WHEN $10 THEN $11 ELSE notes END,
            updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND archived_at IS NULL
        RETURNING id, workspace_id, business_name, billing_email, contact_name,
                  wallet_address, notes, archived_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(access.workspace_id)
    .bind(business_name)
    .bind(billing_email_present)
    .bind(billing_email)
    .bind(contact_name_present)
    .bind(contact_name)
    .bind(customer_wallet_present)
    .bind(customer_wallet_address)
    .bind(notes_present)
    .bind(notes)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Customer not found"))?;
    Ok(Json(CustomerItemResponse { customer }))
}

pub async fn archive_customer(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ArchiveCustomerRequest>,
) -> Result<Json<CustomerItemResponse>, ApiError> {
    let access = customer_access(&state, &body.settings_pda, &body.wallet_address, true).await?;
    let customer = sqlx::query_as::<_, CustomerResponse>(
        r#"
        UPDATE customers
        SET archived_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND workspace_id = $2 AND archived_at IS NULL
        RETURNING id, workspace_id, business_name, billing_email, contact_name,
                  wallet_address, notes, archived_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(access.workspace_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Customer not found"))?;
    Ok(Json(CustomerItemResponse { customer }))
}

#[cfg(test)]
mod tests {
    use super::{normalize_customer_wallet, normalize_email, normalize_required_text};

    #[test]
    fn normalizes_customer_fields() {
        assert_eq!(
            normalize_required_text("  Redwood Holdings  ", "businessName", 160).unwrap(),
            "Redwood Holdings"
        );
        assert_eq!(
            normalize_email(Some(" BILLING@REDWOOD.COM ".to_owned())).unwrap(),
            Some("billing@redwood.com".to_owned())
        );
    }

    #[test]
    fn rejects_invalid_email_and_wallet() {
        assert!(normalize_email(Some("not-an-email".to_owned())).is_err());
        assert!(normalize_customer_wallet(Some("not-a-wallet".to_owned())).is_err());
    }
}
