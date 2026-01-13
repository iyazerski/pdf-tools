use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, Request};
use axum::response::Redirect;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::constants::{GLOBAL_RATE_LIMIT_BURST, GLOBAL_RATE_LIMIT_RPS, MAX_BODY_BYTES};
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
            if let Some(ip) = x_forwarded_for_leftmost(req.headers()) {
                return Ok(ip);
            }
        }

        req.extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip())
            .ok_or(tower_governor::GovernorError::UnableToExtractKey)
    }
}

fn x_forwarded_for_leftmost(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| {
            s.split(',')
                .find_map(|part| part.trim().parse::<std::net::IpAddr>().ok())
        })
}

pub(crate) fn build_router(state: AppState) -> Router {
    let key_extractor = ProxyAwareIpKeyExtractor {
        trust_proxy_headers: state.cookie.trust_proxy_headers,
    };

    let global_governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(key_extractor)
            .per_second(GLOBAL_RATE_LIMIT_RPS)
            .burst_size(GLOBAL_RATE_LIMIT_BURST)
            .finish()
            .expect("governor config must build"),
    );
    let global_governor = GovernorLayer {
        config: global_governor_config.clone(),
    };

    let global_governor_for_health = GovernorLayer {
        config: global_governor_config,
    };

    let health_routes = Router::new()
        .route("/healthz", get(handlers::health::healthz))
        .layer(global_governor_for_health)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::DEBUG))
                .on_response(DefaultOnResponse::new().level(Level::DEBUG))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        );

    let login_routes = Router::new().route("/login", post(handlers::auth::login));
    let auth_routes = Router::new()
        .merge(login_routes)
        .route("/logout", post(handlers::auth::logout));

    let api_routes = Router::new()
        .route("/merge", post(handlers::api::merge))
        .route("/npages", post(handlers::api::npages))
        .route("/page/:upload_id/:page", get(handlers::api::page_png))
        .route("/upload/:upload_id", delete(handlers::api::delete_upload));

    let app_routes = Router::new()
        .route("/", get(handlers::root::index))
        .route_service("/favicon.svg", ServeFile::new("static/favicon.svg"))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/favicon.svg") }),
        )
        .route_service("/og-image.svg", ServeFile::new("static/og-image.svg"))
        .route_service("/robots.txt", ServeFile::new("static/robots.txt"))
        .route_service("/sitemap.xml", ServeFile::new("static/sitemap.xml"))
        .nest_service("/static", ServeDir::new("static"))
        .merge(auth_routes)
        .nest("/api", api_routes)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(global_governor)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        );

    Router::new()
        .merge(health_routes)
        .merge(app_routes)
        .with_state(state)
        .layer(tower_cookies::CookieManagerLayer::new())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use time::Duration;
    use tower::ServiceExt;

    use crate::app::build_router;
    use crate::config::CookieSecureMode;
    use crate::state::AppState;

    fn test_state(trust_proxy_headers: bool) -> AppState {
        AppState::new(
            "test".to_string(),
            "test".to_string(),
            b"secret".to_vec(),
            Duration::hours(1),
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(60),
            CookieSecureMode::Never,
            trust_proxy_headers,
        )
    }

    #[tokio::test]
    async fn healthz_is_rate_limited_by_global_governor() {
        let app = build_router(test_state(true));

        let mut saw_429 = false;
        for _ in 0..500 {
            let req = Request::builder()
                .method("GET")
                .uri("/healthz")
                .header("x-forwarded-for", "1.2.3.4")
                .body(Body::empty())
                .expect("request must build");
            let res = app
                .clone()
                .oneshot(req)
                .await
                .expect("request must succeed");
            if res.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(saw_429, "expected /healthz to eventually be rate limited");
    }

    #[tokio::test]
    async fn login_is_rate_limited_by_global_governor() {
        let app = build_router(test_state(true));

        let mut saw_429 = false;
        for _ in 0..700 {
            let req = Request::builder()
                .method("POST")
                .uri("/login")
                .header("x-forwarded-for", "1.2.3.4")
                .body(Body::empty())
                .expect("request must build");
            let res = app
                .clone()
                .oneshot(req)
                .await
                .expect("request must succeed");
            if res.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(saw_429, "expected /login to eventually be rate limited");
    }

    #[tokio::test]
    async fn api_is_rate_limited_by_global_governor() {
        let app = build_router(test_state(true));

        let mut saw_429 = false;
        for _ in 0..700 {
            let req = Request::builder()
                .method("POST")
                .uri("/api/npages")
                .header("x-forwarded-for", "1.2.3.4")
                .body(Body::empty())
                .expect("request must build");
            let res = app
                .clone()
                .oneshot(req)
                .await
                .expect("request must succeed");
            if res.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "expected /api/npages to eventually be rate limited"
        );
    }
}
