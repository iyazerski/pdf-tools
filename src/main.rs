use std::net::SocketAddr;

use time::Duration;
use tracing::info;

mod app;
mod config;
mod constants;
mod error;
mod handlers;
mod pages;
mod pdf;
mod session;
mod shutdown;
mod state;
mod uploads;
mod util;

use crate::config::AppConfig;
use crate::shutdown::shutdown_signal;
use crate::state::{AppState, AppStateArgs};

#[tokio::main]
async fn main() {
    init_tracing();

    let _ = dotenvy::dotenv();

    let config = AppConfig::from_env();
    let state = AppState::new(AppStateArgs {
        username: config.username,
        password: config.password,
        session_secret: config.session_secret.into_bytes(),
        session_ttl: Duration::hours(24),
        process_timeout: config.process_timeout,
        upload_cache_ttl: config.upload_cache_ttl,
        cookie_secure: config.cookie_secure,
        trust_proxy_headers: config.trust_proxy_headers,
    });

    let app = app::build_router(state);

    info!(bind = %config.bind, "starting server");
    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .expect("bind must succeed");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("server must start");
    info!("server stopped");
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
