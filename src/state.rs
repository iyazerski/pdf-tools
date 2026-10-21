use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use time::{Duration, OffsetDateTime};
use tower_cookies::Cookies;

use crate::config::CookieSecureMode;
use crate::constants::SESSION_COOKIE_NAME;
use crate::constants::UPLOAD_CACHE_MAX_ENTRIES;
use crate::error::AppError;
use crate::session::{Credentials, SessionSigner};
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

/// Shared app state; cloning it per request is a single reference-count bump.
#[derive(Clone)]
pub(crate) struct AppState(Arc<AppStateInner>);

pub(crate) struct AppStateInner {
    pub(crate) credentials: Credentials,
    pub(crate) signer: SessionSigner,
    pub(crate) cookie_secure: CookieSecureMode,
    pub(crate) trust_proxy_headers: bool,
    pub(crate) process_timeout: StdDuration,
    pub(crate) uploads: Arc<UploadStore>,
}

impl Deref for AppState {
    type Target = AppStateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AppState {
    pub(crate) fn new(args: AppStateArgs) -> Self {
        Self(Arc::new(AppStateInner {
            credentials: Credentials::new(
                &args.username,
                &args.password,
                args.session_secret.clone(),
            ),
            signer: SessionSigner::new(args.session_secret, args.session_ttl),
            cookie_secure: args.cookie_secure,
            trust_proxy_headers: args.trust_proxy_headers,
            process_timeout: args.process_timeout,
            uploads: Arc::new(UploadStore::new(
                args.upload_cache_ttl,
                UPLOAD_CACHE_MAX_ENTRIES,
            )),
        }))
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
