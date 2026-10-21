use std::sync::LazyLock;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

// Assets are compiled into the binary: no file I/O per request and nothing to copy at deploy.
const INDEX_TEMPLATE: &str = include_str!("../templates/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLES_CSS: &str = include_str!("../static/styles.css");
const FAVICON_SVG: &str = include_str!("../static/favicon.svg");
const OG_IMAGE_SVG: &str = include_str!("../static/og-image.svg");
const ROBOTS_TXT: &str = include_str!("../static/robots.txt");
const SITEMAP_XML: &str = include_str!("../static/sitemap.xml");

const AUTH_DATA_PLACEHOLDER: &str = "{{AUTH_DATA}}";
const LOGIN_ERROR_DATA_PLACEHOLDER: &str = "{{LOGIN_ERROR_DATA}}";
const ASSET_VERSION_PLACEHOLDER: &str = "{{ASSET_VERSION}}";

// app.js and styles.css URLs carry a content hash, so browsers can cache them forever.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const DAILY: &str = "public, max-age=86400";
// The page embeds the auth state, so it must be revalidated and never shared.
const NO_CACHE: &str = "private, no-cache";

struct IndexPages {
    authed: String,
    anonymous: String,
    login_error: String,
}

static INDEX_PAGES: LazyLock<IndexPages> = LazyLock::new(|| {
    // Fail fast on template drift: it's easy to remove placeholders by accident.
    for placeholder in [
        AUTH_DATA_PLACEHOLDER,
        LOGIN_ERROR_DATA_PLACEHOLDER,
        ASSET_VERSION_PLACEHOLDER,
    ] {
        assert!(
            INDEX_TEMPLATE.contains(placeholder),
            "templates/index.html is missing {placeholder}"
        );
    }

    let mut hasher = Sha256::new();
    hasher.update(APP_JS);
    hasher.update(STYLES_CSS);
    let digest = hasher.finalize();
    let version: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();

    let render = |authed: &str, login_error: &str| {
        INDEX_TEMPLATE
            .replace(ASSET_VERSION_PLACEHOLDER, &version)
            .replace(AUTH_DATA_PLACEHOLDER, authed)
            .replace(LOGIN_ERROR_DATA_PLACEHOLDER, login_error)
    };
    IndexPages {
        authed: render("1", "0"),
        anonymous: render("0", "0"),
        login_error: render("0", "1"),
    }
});

/// Render all index page variants now, so a broken template fails at startup.
pub(crate) fn init() {
    LazyLock::force(&INDEX_PAGES);
}

/// Return the pre-rendered index page for the given auth state.
pub(crate) fn index_page(is_authed: bool, show_login_error: bool) -> Response {
    let pages = &*INDEX_PAGES;
    let body = match (is_authed, show_login_error) {
        (true, _) => pages.authed.as_str(),
        (false, true) => pages.login_error.as_str(),
        (false, false) => pages.anonymous.as_str(),
    };
    asset(body, "text/html; charset=utf-8", NO_CACHE)
}

fn asset(body: &'static str, content_type: &'static str, cache: &'static str) -> Response {
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn app_js() -> Response {
    asset(APP_JS, "text/javascript; charset=utf-8", IMMUTABLE)
}

pub(crate) async fn styles_css() -> Response {
    asset(STYLES_CSS, "text/css; charset=utf-8", IMMUTABLE)
}

pub(crate) async fn favicon_svg() -> Response {
    asset(FAVICON_SVG, "image/svg+xml", DAILY)
}

pub(crate) async fn og_image_svg() -> Response {
    asset(OG_IMAGE_SVG, "image/svg+xml", DAILY)
}

pub(crate) async fn robots_txt() -> Response {
    asset(ROBOTS_TXT, "text/plain; charset=utf-8", DAILY)
}

pub(crate) async fn sitemap_xml() -> Response {
    asset(SITEMAP_XML, "application/xml", DAILY)
}

#[cfg(test)]
mod tests {
    #[test]
    fn index_pages_render_without_placeholders() {
        super::init();
        let pages = &*super::INDEX_PAGES;
        for page in [&pages.authed, &pages.anonymous, &pages.login_error] {
            assert!(!page.contains("{{"), "unrendered placeholder in index page");
        }
        assert!(pages.authed.contains(r#"data-authed="1""#));
        assert!(pages.login_error.contains(r#"data-login-error="1""#));
    }
}
