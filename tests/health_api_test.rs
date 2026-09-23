use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use myagentrust::create_app;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok_json() {
    let app = create_app();

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
    let app = create_app();

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
    let response = create_app()
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
