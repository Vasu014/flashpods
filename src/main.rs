use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::{from_fn, Next},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

mod artifacts;
mod db;
mod jobs;
mod middleware;
mod models;
mod podman;
mod uploads;

use db::{Database, JobRepository, UploadRepository};
use models::UploadConfig;
use podman::PodmanService;

/// Application state
#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub upload_repo: Arc<UploadRepository>,
    pub job_repo: Arc<JobRepository>,
    pub upload_config: UploadConfig,
    pub podman: Arc<PodmanService>,
    pub start_time: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flashpods_api=debug,tower_http=debug,axum=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize database with migrations
    let db = db::init_db("flashpods.db").await?;
    info!("Database initialized");

    let upload_repo = Arc::new(UploadRepository::new(db.inner().clone()));
    let job_repo = Arc::new(JobRepository::new(db.inner().clone()));
    let upload_config = UploadConfig::default();
    let podman = Arc::new(PodmanService::new());
    let start_time = Instant::now();

    // Check podman availability
    if podman.is_available() {
        let version = podman.version().unwrap_or_else(|_| "unknown".to_string());
        info!("Podman available: {}", version);
    } else {
        tracing::warn!("Podman not available - container operations will fail");
    }

    let state = AppState {
        db,
        upload_repo,
        job_repo,
        upload_config,
        podman,
        start_time,
    };

    let app = Router::new()
        .route("/health", get(health))
        .nest("/uploads", uploads::routes())
        .nest("/jobs", jobs::routes())
        .nest("/artifacts", artifacts::routes())
        .layer(from_fn(request_headers))
        .layer(from_fn(middleware::auth_middleware))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Health endpoint - no auth required
async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    uptime_seconds: u64,
}

/// Middleware to add X-Request-Id and rate limiting headers
async fn request_headers(request: Request, next: Next) -> impl IntoResponse {
    let request_id = Uuid::new_v4().to_string();

    // Run the handler
    let mut response = next.run(request).await;

    // Add headers to response
    let headers = response.headers_mut();
    headers.insert(
        "X-Request-Id",
        HeaderValue::from_str(&request_id).unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    headers.insert("X-RateLimit-Limit", HeaderValue::from_static("100"));
    headers.insert("X-RateLimit-Remaining", HeaderValue::from_static("95"));
    headers.insert("X-RateLimit-Reset", HeaderValue::from_static("0"));

    response
}
