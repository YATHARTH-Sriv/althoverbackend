use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{
        HeaderName, HeaderValue, Method,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
    routing::{get, patch, post},
};
use tower_http::cors::CorsLayer;

use crate::{
    AppState,
    handlers::{
        accept_invite, add_signer_submitted, agent_program_config, approval_submitted,
        auth_challenge, auth_verify, build_add_signer, build_approve_transaction,
        build_change_threshold, build_create_proposal, build_create_settings,
        build_create_transaction, build_execute_transaction, build_fund_smart_account,
        build_remove_signer, change_threshold_submitted, create_extension_session,
        create_invite_challenge, create_smart_account, dashboard, delete_receipt,
        exchange_extension_session, execute_submitted, extension_scan_receipt,
        fund_smart_account_submitted, get_extension_session, get_invite, health, receipt_file,
        receipt_payment_submitted, refresh_settings, refresh_smart_account, refresh_transaction,
        remove_signer_submitted, revoke_extension_session, scan_receipt, send_signed_transaction,
        settings_agent_chat, settings_submitted, update_receipt, wallet_transaction_submitted,
    },
};

pub fn create_app(state: AppState) -> Router {
    let mut allowed_origins = vec![
        "http://localhost:3000"
            .parse::<HeaderValue>()
            .expect("valid frontend origin"),
        "http://127.0.0.1:3000"
            .parse::<HeaderValue>()
            .expect("valid frontend origin"),
    ];
    allowed_origins.extend(
        std::env::var("EXTENSION_IDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .filter_map(|id| {
                format!("chrome-extension://{id}")
                    .parse::<HeaderValue>()
                    .ok()
            }),
    );
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static("x-file-name"),
            HeaderName::from_static("x-hover-source-url"),
            HeaderName::from_static("x-hover-source-title"),
            HeaderName::from_static("x-hover-smart-account-id"),
        ]);
    Router::new()
        .route("/health", get(health))
        .route("/internal/agent/program-config", get(agent_program_config))
        .route("/agent/settings/chat", post(settings_agent_chat))
        .route("/auth/challenge", post(auth_challenge))
        .route("/auth/verify", post(auth_verify))
        .route("/auth/extension-session", post(create_extension_session))
        .route(
            "/auth/extension-session/revoke",
            post(revoke_extension_session),
        )
        .route(
            "/auth/extension-session/exchange",
            post(exchange_extension_session),
        )
        .route("/extension/session", get(get_extension_session))
        .route("/invites/{token}", get(get_invite))
        .route("/invites/{token}/challenge", post(create_invite_challenge))
        .route("/invites/{token}/accept", post(accept_invite))
        .route("/receipts/scan", post(scan_receipt))
        .route("/extension/receipts/scan", post(extension_scan_receipt))
        .route(
            "/receipts/{id}",
            patch(update_receipt).delete(delete_receipt),
        )
        .route("/receipts/{id}/file", get(receipt_file))
        .route(
            "/receipts/{id}/payment-submitted",
            post(receipt_payment_submitted),
        )
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
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .layer(cors)
        .with_state(state)
}
