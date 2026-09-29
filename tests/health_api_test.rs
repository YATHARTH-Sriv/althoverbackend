use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use myagentrust::{AppState, create_app, solanasetup::create_rpc_client};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn test_app() -> axum::Router {
    let db = PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost/test")
        .expect("test database URL should be valid");
    create_app(AppState {
        db,
        rpc: create_rpc_client("http://127.0.0.1:8899"),
        http: reqwest::Client::new(),
    })
}

#[tokio::test]
async fn health_returns_ok_json() {
    let app = test_app();

    let request = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(body, serde_json::json!({ "ok": true }));
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let app = test_app();

    let request = Request::builder()
        .method("GET")
        .uri("/random")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_rejects_unsupported_method() {
    let response = test_app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
