mod api;
mod auth;
mod config;
mod endpoints;
mod job_endpoints;
mod jobs;
mod models;
mod state;
mod stream;
mod telemetry;

use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
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
        .route("/admin", get(serve_admin_console))
        .route("/auth/login", post(endpoints::login))
        .route("/auth/logout", post(endpoints::logout))
        .route("/auth/me", get(endpoints::get_current_user))
        .route("/admin/users", post(endpoints::create_user).get(endpoints::list_users))
        .route("/admin/users/{id}", put(endpoints::update_user).delete(endpoints::delete_user))
        .route("/admin/users/{id}/password", put(endpoints::change_password))
        .route("/api/v2/jobs/submit", post(job_endpoints::submit_job))
        .route("/api/v2/jobs", get(job_endpoints::list_jobs))
        .route("/api/v2/jobs/{id}", get(job_endpoints::get_job))
        .route("/api/v2/jobs/queue", get(job_endpoints::get_queue))
        .route("/api/v2/jobs/{id}/cancel", post(job_endpoints::cancel_job))
        .route("/api/v2/jobs/{id}/dispatch/{printer_id}", post(job_endpoints::dispatch_job))
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
        .with_state(state)
        .into_make_service_with_connect_info::<std::net::SocketAddr>();

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("failed to bind API listener");

    info!("listening on {}", bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server failed");
}

async fn serve_admin_console() -> impl IntoResponse {
    let html = include_str!("static/admin.html");
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
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
