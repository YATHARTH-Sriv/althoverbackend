use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    ok: bool,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}
