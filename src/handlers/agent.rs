use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode, header},
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    ApiError, AppState, internal_error, squadsaccounts::fetch_program_config, unauthorized,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramConfigResponse {
    program_config_pda: String,
    smart_account_index: String,
    authority: String,
    smart_account_creation_fee: String,
    treasury: String,
}

fn internal_agent_url() -> String {
    let base =
        std::env::var("NODE_INTERNAL_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_owned());
    format!(
        "{}/internal/agent/settings/chat",
        base.trim_end_matches('/')
    )
}

fn internal_token() -> Result<String, ApiError> {
    std::env::var("INTERNAL_SERVICE_TOKEN")
        .map_err(|_| internal_error("INTERNAL_SERVICE_TOKEN is required"))
}

fn require_internal_token(headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = internal_token()?;
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    let mut difference = expected.len() ^ provided.len();
    for index in 0..expected.len().max(provided.len()) {
        difference |= usize::from(
            expected.get(index).copied().unwrap_or_default()
                ^ provided.get(index).copied().unwrap_or_default(),
        );
    }
    if difference != 0 {
        return Err(unauthorized("Unauthorized internal request"));
    }
    Ok(())
}

pub async fn agent_program_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProgramConfigResponse>, ApiError> {
    require_internal_token(&headers)?;
    let (program_config_pda, config) = fetch_program_config(state.rpc.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(ProgramConfigResponse {
        program_config_pda: program_config_pda.to_string(),
        smart_account_index: config.smart_account_index.to_string(),
        authority: config.authority.to_string(),
        smart_account_creation_fee: config.smart_account_creation_fee.to_string(),
        treasury: config.treasury.to_string(),
    }))
}

pub async fn settings_agent_chat(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Response<Body>, ApiError> {
    let response = state
        .http
        .post(internal_agent_url())
        .bearer_auth(internal_token()?)
        .json(&payload)
        .send()
        .await
        .map_err(internal_error)?;

    let status = StatusCode::from_u16(response.status().as_u16()).map_err(internal_error)?;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("application/json"));
    let body = response.bytes().await.map_err(internal_error)?;

    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .map_err(internal_error)
}
