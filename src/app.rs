use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    AppState,
    handlers::{auth_challenge, auth_verify, health},
};

pub fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/auth/challenge", post(auth_challenge))
        .route("/auth/verify", post(auth_verify))
        .with_state(state)
}
