use std::sync::Arc;
use std::time::Duration as StdDuration;

use time::{Duration, OffsetDateTime};
use tower_cookies::Cookies;

use crate::config::CookieSecureMode;
use crate::constants::SESSION_COOKIE_NAME;
use crate::constants::UPLOAD_CACHE_MAX_ENTRIES;
use crate::error::AppError;
use crate::session::SessionSigner;
use crate::uploads::UploadStore;

pub(crate) struct AppStateArgs {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) session_secret: Vec<u8>,
    pub(crate) session_ttl: Duration,
    pub(crate) process_timeout: StdDuration,
    pub(crate) upload_cache_ttl: StdDuration,
    pub(crate) cookie_secure: CookieSecureMode,
    pub(crate) trust_proxy_headers: bool,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) auth: Arc<AuthConfig>,
    pub(crate) signer: Arc<SessionSigner>,
    pub(crate) cookie: Arc<CookieConfig>,
    pub(crate) process_timeout: StdDuration,
    pub(crate) uploads: Arc<UploadStore>,
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
    pub(crate) fn new(args: AppStateArgs) -> Self {
        Self {
            auth: Arc::new(AuthConfig {
                username: args.username,
                password: args.password,
            }),
            signer: Arc::new(SessionSigner::new(args.session_secret, args.session_ttl)),
            cookie: Arc::new(CookieConfig {
                secure: args.cookie_secure,
                trust_proxy_headers: args.trust_proxy_headers,
            }),
            process_timeout: args.process_timeout,
            uploads: Arc::new(UploadStore::new(
                args.upload_cache_ttl,
                UPLOAD_CACHE_MAX_ENTRIES,
            )),
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
