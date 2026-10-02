use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, header},
};
use uuid::Uuid;

use crate::{ApiError, AppState, bad_request, internal_error};

use super::models::NodeProcessedReceipt;

const MAX_RECEIPT_BYTES: usize = 20 * 1024 * 1024;

fn internal_token(state: &AppState) -> Result<&str, ApiError> {
    state
        .config
        .internal_service_token
        .as_deref()
        .ok_or_else(|| internal_error("INTERNAL_SERVICE_TOKEN is required"))
}

pub(super) fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
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

pub(super) async fn process_file(
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

pub(super) async fn delete_file(state: &AppState, pathname: &str) -> Result<(), ApiError> {
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

pub(super) async fn fetch_file(
    state: &AppState,
    pathname: &str,
) -> Result<(HeaderValue, Bytes), ApiError> {
    let response = state
        .http
        .get(state.config.node_url("/internal/receipts/file"))
        .bearer_auth(internal_token(state)?)
        .query(&[("pathname", pathname)])
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
    Ok((content_type, bytes))
}
