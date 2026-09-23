use axum::{Router, routing::get};

use crate::handlers::health;

pub fn create_app() -> Router {
    Router::new().route("/health", get(health))
}
