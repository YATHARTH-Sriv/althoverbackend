use axum::{
    Router,
    http::{HeaderValue, Method, header::CONTENT_TYPE},
    routing::{get, post},
};
use tower_http::cors::CorsLayer;

use crate::{
    AppState,
    handlers::{
        add_signer_submitted, approval_submitted, auth_challenge, auth_verify, build_add_signer,
        build_approve_transaction, build_change_threshold, build_create_proposal,
        build_create_settings, build_create_transaction, build_execute_transaction,
        build_fund_smart_account, build_remove_signer, change_threshold_submitted,
        create_smart_account, dashboard, execute_submitted, fund_smart_account_submitted, health,
        refresh_settings, refresh_smart_account, refresh_transaction, remove_signer_submitted,
        send_signed_transaction, settings_submitted, wallet_transaction_submitted,
    },
};

pub fn create_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(
            "http://localhost:3000"
                .parse::<HeaderValue>()
                .expect("valid frontend origin"),
        )
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([CONTENT_TYPE]);
    Router::new()
        .route("/health", get(health))
        .route("/auth/challenge", post(auth_challenge))
        .route("/auth/verify", post(auth_verify))
        .route("/dashboard", get(dashboard))
        .route("/settings/build-create", post(build_create_settings))
        .route("/settings/submitted", post(settings_submitted))
        .route(
            "/settings/change-threshold/build",
            post(build_change_threshold),
        )
        .route(
            "/settings/change-threshold/submitted",
            post(change_threshold_submitted),
        )
        .route("/settings/add-signer/build", post(build_add_signer))
        .route("/settings/add-signer/submitted", post(add_signer_submitted))
        .route("/settings/remove-signer/build", post(build_remove_signer))
        .route(
            "/settings/remove-signer/submitted",
            post(remove_signer_submitted),
        )
        .route("/settings/{id}/refresh", post(refresh_settings))
        .route("/smart-accounts", post(create_smart_account))
        .route("/smart-accounts/fund/build", post(build_fund_smart_account))
        .route(
            "/smart-accounts/fund/submitted",
            post(fund_smart_account_submitted),
        )
        .route("/smart-accounts/{id}/refresh", post(refresh_smart_account))
        .route("/transactions/send", post(send_signed_transaction))
        .route("/transactions/build-create", post(build_create_transaction))
        .route("/transactions/build-proposal", post(build_create_proposal))
        .route(
            "/transactions/wallet-submitted",
            post(wallet_transaction_submitted),
        )
        .route("/transactions/{id}/refresh", post(refresh_transaction))
        .route(
            "/transactions/{id}/approve-transaction",
            get(build_approve_transaction),
        )
        .route(
            "/transactions/{id}/approval-submitted",
            post(approval_submitted),
        )
        .route(
            "/transactions/{id}/execute-transaction",
            get(build_execute_transaction),
        )
        .route(
            "/transactions/{id}/execute-submitted",
            post(execute_submitted),
        )
        .layer(cors)
        .with_state(state)
}
