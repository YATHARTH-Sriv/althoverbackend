use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::{
    models::{InvoiceLineItemResponse, InvoicePaymentResponse, InvoiceResponse, InvoiceRow},
    service::token_hash,
};

pub(super) async fn load_invoice(
    state: &AppState,
    workspace_id: Uuid,
    id: Uuid,
) -> Result<InvoiceResponse, ApiError> {
    let invoice = sqlx::query_as::<_, InvoiceRow>(
        r#"SELECT id, workspace_id, customer_id,
        receiving_smart_account_id, invoice_number, status, currency, issue_date, due_date,
        customer_business_name, customer_billing_email, customer_contact_name,
        customer_wallet_address, memo, subtotal::text AS subtotal, total::text AS total,
        amount_paid::text AS amount_paid, created_by_wallet, created_at, updated_at,
        sent_at, viewed_at FROM invoices WHERE id = $1 AND workspace_id = $2"#,
    )
    .bind(id)
    .bind(workspace_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invoice not found"))?;
    let line_items = sqlx::query_as::<_, InvoiceLineItemResponse>(
        r#"SELECT id, position,
        description, quantity::text AS quantity, unit_price::text AS unit_price,
        amount::text AS amount FROM invoice_line_items WHERE invoice_id = $1 ORDER BY position"#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal_error)?;
    let payments = sqlx::query_as::<_, InvoicePaymentResponse>(
        r#"SELECT id, payer_wallet,
        amount_lamports::text AS amount_lamports, tx_sig, confirmed_at
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

pub(super) async fn public_invoice_context(
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
    let row = sqlx::query_as::<_, (Uuid, Uuid, String, String, String)>(
        r#"SELECT i.id,
        i.workspace_id, w.name, sa.name, sa.pda FROM invoices i
        JOIN workspaces w ON w.id = i.workspace_id
        JOIN smart_accounts sa ON sa.id = i.receiving_smart_account_id
        WHERE i.public_token_hash = $1 AND i.status NOT IN ('DRAFT', 'VOID')"#,
    )
    .bind(token_hash(token))
    .fetch_optional(&state.db)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| bad_request("Invoice link is invalid or no longer available"))?;
    if mark_viewed {
        sqlx::query(
            r#"UPDATE invoices SET status = CASE WHEN status='SENT' THEN 'VIEWED'
            ELSE status END, viewed_at = COALESCE(viewed_at, NOW()), updated_at=NOW()
            WHERE id=$1"#,
        )
        .bind(row.0)
        .execute(&state.db)
        .await
        .map_err(internal_error)?;
    }
    Ok(row)
}
