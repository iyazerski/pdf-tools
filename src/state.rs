use std::sync::Arc;
use std::time::Duration as StdDuration;

use time::{Duration, OffsetDateTime};
use tower_cookies::Cookies;

use crate::config::CookieSecureMode;
use crate::constants::SESSION_COOKIE_NAME;
use crate::error::AppError;
use crate::session::SessionSigner;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) auth: Arc<AuthConfig>,
    pub(crate) signer: Arc<SessionSigner>,
    pub(crate) cookie: Arc<CookieConfig>,
    pub(crate) process_timeout: StdDuration,
}

pub(crate) struct AuthConfig {
    pub(crate) username: String,
    pub(crate) password: String,
}

pub(crate) struct CookieConfig {
    pub(crate) secure: CookieSecureMode,
    pub(crate) trust_proxy_headers: bool,
}

impl AppState {
    pub(crate) fn new(
        username: String,
        password: String,
        session_secret: Vec<u8>,
        session_ttl: Duration,
        process_timeout: StdDuration,
        cookie_secure: CookieSecureMode,
        trust_proxy_headers: bool,
    ) -> Self {
        Self {
            auth: Arc::new(AuthConfig { username, password }),
            signer: Arc::new(SessionSigner::new(session_secret, session_ttl)),
            cookie: Arc::new(CookieConfig {
                secure: cookie_secure,
                trust_proxy_headers,
            }),
            process_timeout,
        }
    }

    pub(crate) fn authed_username(&self, cookies: &Cookies) -> Option<String> {
        let token = cookies.get(SESSION_COOKIE_NAME)?;
        let now = OffsetDateTime::now_utc();
        self.signer.verify(token.value(), now).map(|p| p.u)
    }

    pub(crate) fn require_auth(&self, cookies: &Cookies) -> Result<String, AppError> {
        self.authed_username(cookies).ok_or(AppError::Unauthorized)
    }
}
