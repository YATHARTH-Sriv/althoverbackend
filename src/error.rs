use axum::{Json, http::StatusCode};
use serde::Serialize;

#[derive(Serialize)]
pub struct ErrorBody {
    error: String,
}

pub type ApiError = (StatusCode, Json<ErrorBody>);

pub fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
}

pub fn internal_error(error: impl std::fmt::Display) -> ApiError {
    eprintln!("internal server error: {error}");

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "Internal server error".to_owned(),
        }),
    )
}
