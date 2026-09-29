use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::{
    ApiError, AppState, bad_request, internal_error, solanasetup::decode_signed_transaction,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTransactionRequest {
    pub transaction_base64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendTransactionResponse {
    pub tx_sig: String,
}

pub async fn send_signed_transaction(
    State(state): State<AppState>,
    Json(payload): Json<SendTransactionRequest>,
) -> Result<Json<SendTransactionResponse>, ApiError> {
    if payload.transaction_base64.trim().is_empty() {
        return Err(bad_request("transactionBase64 is required"));
    }

    let transaction =
        decode_signed_transaction(&payload.transaction_base64).map_err(bad_request)?;

    let signature = state
        .rpc
        .send_and_confirm_transaction(&transaction)
        .await
        .map_err(internal_error)?;

    Ok(Json(SendTransactionResponse {
        tx_sig: signature.to_string(),
    }))
}
