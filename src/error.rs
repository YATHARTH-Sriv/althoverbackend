use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    NotFound(String),
    Internal,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    error: String,
    code: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, "unauthorized", message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error".to_owned(),
            ),
        };

        (
            status,
            Json(ErrorBody {
                error: message,
                code,
            }),
        )
            .into_response()
    }
}

pub fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError::BadRequest(message.into())
}

pub fn unauthorized(message: impl Into<String>) -> ApiError {
    ApiError::Unauthorized(message.into())
}

pub fn not_found(message: impl Into<String>) -> ApiError {
    ApiError::NotFound(message.into())
}

pub fn internal_error(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %error, "request failed");
    ApiError::Internal
}
