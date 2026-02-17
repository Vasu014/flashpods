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

mod jobs;
mod uploads;
mod artifacts;
mod db;
mod models;

use db::{Database, UploadRepository};
use models::UploadConfig;

#[derive(Clone)]
pub struct AppState {
    db: Database,
    upload_repo: Arc<UploadRepository>,
    upload_config: UploadConfig,
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
    let upload_config = UploadConfig::default();

    let state = AppState {
        db,
        upload_repo,
        upload_config,
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
