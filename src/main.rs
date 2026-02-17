use axum::{
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod artifacts;
mod db;
mod jobs;
mod models;
mod podman;
mod uploads;

use db::{Database, JobRepository, UploadRepository};
use models::UploadConfig;
use podman::PodmanService;

#[derive(Clone)]
pub struct AppState {
    db: Database,
    upload_repo: Arc<UploadRepository>,
    job_repo: Arc<JobRepository>,
    upload_config: UploadConfig,
    podman: Arc<PodmanService>,
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
    };

    let app = Router::new()
        .route("/health", get(health))
        .nest("/uploads", uploads::routes())
        .nest("/jobs", jobs::routes())
        .nest("/artifacts", artifacts::routes())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on {}", addr);

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}
