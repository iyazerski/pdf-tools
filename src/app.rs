use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, Request};
use axum::routing::{get, post};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::constants::MAX_BODY_BYTES;
use crate::handlers;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProxyAwareIpKeyExtractor {
    trust_proxy_headers: bool,
}

impl KeyExtractor for ProxyAwareIpKeyExtractor {
    type Key = std::net::IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, tower_governor::GovernorError> {
        if self.trust_proxy_headers {
            if let Some(ip) = x_forwarded_for_rightmost(req.headers()) {
                return Ok(ip);
            }
        }

        req.extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip())
            .ok_or(tower_governor::GovernorError::UnableToExtractKey)
    }
}

fn x_forwarded_for_rightmost(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| {
            s.split(',')
                .rev()
                .find_map(|part| part.trim().parse::<std::net::IpAddr>().ok())
        })
}

pub(crate) fn build_router(state: AppState) -> Router {
    let key_extractor = ProxyAwareIpKeyExtractor {
        trust_proxy_headers: state.cookie.trust_proxy_headers,
    };

    let global_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(key_extractor)
                .per_second(5)
                .burst_size(10)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let login_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(key_extractor)
                .per_second(1)
                .burst_size(3)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let api_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(key_extractor)
                .per_second(2)
                .burst_size(10)
                .finish()
                .expect("governor config must build"),
        ),
    };

    let login_routes = Router::new()
        .route("/login", post(handlers::auth::login))
        .route_layer(login_governor);
    let auth_routes = Router::new()
        .merge(login_routes)
        .route("/logout", post(handlers::auth::logout));

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
