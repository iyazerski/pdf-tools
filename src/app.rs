use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::constants::MAX_BODY_BYTES;
use crate::handlers;
use crate::state::AppState;

pub(crate) fn build_router(state: AppState) -> Router {
    let global_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(10)
                .burst_size(30)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let auth_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(1)
                .burst_size(5)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let api_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(2)
                .burst_size(10)
                .finish()
                .expect("governor config must build"),
        ),
    };

    let auth_routes = Router::new()
        .route("/login", post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route_layer(auth_governor);

    let api_routes = Router::new()
        .route("/merge", post(handlers::api::merge))
        .route("/npages", post(handlers::api::npages))
        .route_layer(api_governor);

    Router::new()
        .route("/", get(handlers::root::index))
        .route("/healthz", get(handlers::health::healthz))
        .merge(auth_routes)
        .nest("/api", api_routes)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .with_state(state)
        .layer(tower_cookies::CookieManagerLayer::new())
        .layer(global_governor)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
}
