use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;
use time::OffsetDateTime;
use tower_cookies::cookie::SameSite;
use tower_cookies::{Cookie, Cookies};

use crate::constants::SESSION_COOKIE_NAME;
use crate::error::AppError;
use crate::pages::render_login_page;
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    pub(crate) username: String,
    pub(crate) password: String,
}

pub(crate) async fn login(
    State(state): State<AppState>,
    cookies: Cookies,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let expected_username = state.auth.username.as_str();
    let expected_password = state.auth.password.as_str();
    if form.username != expected_username || form.password != expected_password {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Html(render_login_page(Some("Invalid username or password."))),
        )
            .into_response());
    }

    let token = state
        .signer
        .issue(&form.username, OffsetDateTime::now_utc());
    let mut cookie = Cookie::new(SESSION_COOKIE_NAME, token);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(cookie_should_be_secure(&state, &headers));
    cookies.add(cookie);

    Ok(Redirect::to("/").into_response())
}

pub(crate) async fn logout(
    State(state): State<AppState>,
    cookies: Cookies,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let _ = state.authed_username(&cookies);
    let mut cookie = Cookie::new(SESSION_COOKIE_NAME, "");
    cookie.set_path("/");
    cookie.set_secure(cookie_should_be_secure(&state, &headers));
    cookie.make_removal();
    cookies.add(cookie);
    Ok(Redirect::to("/").into_response())
}

fn cookie_should_be_secure(state: &AppState, headers: &HeaderMap) -> bool {
    match state.cookie.secure {
        crate::config::CookieSecureMode::Always => true,
        crate::config::CookieSecureMode::Never => false,
        crate::config::CookieSecureMode::Auto => {
            state.cookie.trust_proxy_headers && forwarded_proto_is_https(headers)
        }
    }
}

fn forwarded_proto_is_https(headers: &HeaderMap) -> bool {
    if let Some(v) = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
    {
        let first = v.split(',').next().unwrap_or("").trim();
        if first.eq_ignore_ascii_case("https") {
            return true;
        }
    }

    if let Some(v) = headers.get("forwarded").and_then(|v| v.to_str().ok()) {
        let v = v.to_ascii_lowercase();
        if v.contains("proto=https") {
            return true;
        }
    }

    false
}
