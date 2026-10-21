use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, Request};
use axum::response::Redirect;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::assets;
use crate::constants::{LOGIN_RATE_LIMIT_BURST, LOGIN_RATE_LIMIT_PERIOD, MAX_BODY_BYTES};
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
        trust_proxy_headers: state.trust_proxy_headers,
    };

    // Only login needs throttling (password guessing); limiting the API would break bursts of
    // thumbnail requests. Note: the builder's `period` is the time to replenish one request.
    let login_governor_config = GovernorConfigBuilder::default()
        .key_extractor(key_extractor)
        .period(LOGIN_RATE_LIMIT_PERIOD)
        .burst_size(LOGIN_RATE_LIMIT_BURST)
        .finish()
        .expect("governor config must build");
    let login_governor = GovernorLayer::new(login_governor_config);

    // Probes and static assets are routed outside the request trace layer to keep logs useful.
    let untraced_routes = Router::new()
        .route("/healthz", get(handlers::health::healthz))
        .route("/static/app.js", get(assets::app_js))
        .route("/static/styles.css", get(assets::styles_css))
        .route("/favicon.svg", get(assets::favicon_svg))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::permanent("/favicon.svg") }),
        )
        .route("/og-image.svg", get(assets::og_image_svg))
        .route("/robots.txt", get(assets::robots_txt))
        .route("/sitemap.xml", get(assets::sitemap_xml));

    let api_routes = Router::new()
        .route("/merge", post(handlers::api::merge))
        .route("/npages", post(handlers::api::npages))
        .route("/page/{upload_id}/{page}", get(handlers::api::page_image))
        .route("/upload/{upload_id}", delete(handlers::api::delete_upload));

    let app_routes = Router::new()
        .route("/", get(handlers::root::index))
        .route("/login", post(handlers::auth::login).layer(login_governor))
        .route("/logout", post(handlers::auth::logout))
        .nest("/api", api_routes)
        // Enforced by the Multipart, Json and Form extractors.
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .layer(tower_cookies::CookieManagerLayer::new());

    Router::new()
        .merge(untraced_routes)
        .merge(app_routes)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use time::Duration;
    use tower::ServiceExt;

    use crate::app::build_router;
    use crate::config::CookieSecureMode;
    use crate::state::{AppState, AppStateArgs};

    fn test_state(trust_proxy_headers: bool) -> AppState {
        AppState::new(AppStateArgs {
            username: "test".to_string(),
            password: "test".to_string(),
            session_secret: b"secret".to_vec(),
            session_ttl: Duration::hours(1),
            process_timeout: std::time::Duration::from_secs(30),
            upload_cache_ttl: std::time::Duration::from_secs(60),
            cookie_secure: CookieSecureMode::Never,
            trust_proxy_headers,
        })
    }

    async fn first_429(app: &axum::Router, method: &str, uri: &str, attempts: usize) -> bool {
        for _ in 0..attempts {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("x-forwarded-for", "1.2.3.4")
                .body(Body::empty())
                .expect("request must build");
            let res = app
                .clone()
                .oneshot(req)
                .await
                .expect("request must succeed");
            if res.status() == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }
        }
        false
    }

    #[tokio::test]
    async fn healthz_is_not_rate_limited() {
        let app = build_router(test_state(true));
        assert!(
            !first_429(&app, "GET", "/healthz", 500).await,
            "expected /healthz to never be rate limited (it is used by liveness/readiness probes)"
        );
    }

    #[tokio::test]
    async fn login_is_rate_limited() {
        let app = build_router(test_state(true));
        assert!(
            first_429(&app, "POST", "/login", 20).await,
            "expected /login to be rate limited"
        );
    }

    #[tokio::test]
    async fn api_is_not_rate_limited() {
        let app = build_router(test_state(true));
        assert!(
            !first_429(&app, "GET", "/api/page/x/1", 500).await,
            "expected /api to never be rate limited (thumbnails load in bursts)"
        );
    }
}
