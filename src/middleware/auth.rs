use axum::{
    body::Body,
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::env;

/// Bearer token authentication middleware
pub async fn auth_middleware(request: Request, next: Next) -> Response {
    // Skip auth for health endpoint
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    // Get expected token from environment
    let expected_token = match env::var("FLASHPODS_API_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            tracing::error!("FLASHPODS_API_TOKEN not configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "server_misconfigured",
                    "message": "API token not configured"
                })),
            )
                .into_response();
        }
    };

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(header) => {
            // Check Bearer format
            let parts: Vec<&str> = header.splitn(2, ' ').collect();
            if parts.len() != 2 || parts[0] != "Bearer" {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "invalid_auth_format",
                        "message": "Authorization header must be 'Bearer <token>'"
                    })),
                )
                    .into_response();
            }

            // Validate token
            if parts[1] != expected_token {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "invalid_token",
                        "message": "Invalid or expired token"
                    })),
                )
                    .into_response();
            }

            // Token valid, proceed
            next.run(request).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "missing_auth",
                "message": "Authorization header required"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{Method, Request},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    fn setup_test_app() -> Router {
        unsafe {
            env::set_var("FLASHPODS_API_TOKEN", "test-token-123");
        }

        Router::new()
            .route("/protected", get(|| async { "ok" }))
            .route("/health", get(|| async { "healthy" }))
            .layer(middleware::from_fn(auth_middleware))
    }

    #[tokio::test]
    async fn test_health_no_auth_required() {
        let app = setup_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_protected_missing_auth() {
        let app = setup_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_protected_invalid_token() {
        let app = setup_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/protected")
                    .header(AUTHORIZATION, "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_protected_valid_token() {
        let app = setup_test_app();

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/protected")
                    .header(AUTHORIZATION, "Bearer test-token-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
