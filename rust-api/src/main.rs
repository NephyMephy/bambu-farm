mod api;
mod config;
mod models;
mod state;
mod stream;
mod telemetry;

use axum::routing::{get, post};
use axum::Router;
use state::AppState;
use tracing::info;

#[tokio::main]
async fn main() {
    // Install the rustls crypto provider (needed for proprietary TLS streaming)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let settings = config::Settings::from_env();
    let bind_addr = settings.bind_addr.clone();
    let state = AppState::new(settings);
    state.start_telemetry().await;

    let app = Router::new()
        .route("/health", get(api::health))
        .route("/", get(api::dashboard))
        .route("/v1/printers", post(api::upsert_printer).get(api::list_printers))
        .route("/v1/printers/batch", post(api::batch_upsert_printers))
        .route("/v1/printers/{id}", get(api::get_printer).delete(api::delete_printer))
        .route("/v1/printers/{id}/stream/start", post(api::start_stream))
        .route("/v1/printers/{id}/stream/stop", post(api::stop_stream))
        .route("/v1/printers/{id}/stream/url", get(api::stream_url))
        .route("/v1/printers/{id}/stream/snapshot", get(api::stream_snapshot))
        .route("/v1/printers/{id}/stream/mjpeg", get(api::stream_mjpeg))
        .route("/v1/streams/start", post(api::start_all_streams))
        .route("/v1/streams/stop", post(api::stop_all_streams))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind API listener");

    info!("listening on {}", bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("received Ctrl+C, shutting down"),
        _ = terminate => info!("received SIGTERM, shutting down"),
    }
}
