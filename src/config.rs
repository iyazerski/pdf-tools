use std::env;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CookieSecureMode {
    Auto,
    Always,
    Never,
}

pub(crate) struct AppConfig {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) session_secret: String,
    pub(crate) bind: String,
    pub(crate) process_timeout: Duration,
    pub(crate) cookie_secure: CookieSecureMode,
    pub(crate) trust_proxy_headers: bool,
}

impl AppConfig {
    pub(crate) fn from_env() -> Self {
        let username = required_env_non_empty("APP_USERNAME");
        let password = required_env_non_empty("APP_PASSWORD");
        let session_secret = required_env_non_empty("SESSION_SECRET");
        let bind = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8091".to_string());
        let process_timeout = env_u64_or("EXTERNAL_PROCESS_TIMEOUT_SECS", 120);
        let cookie_secure = cookie_secure_mode_from_env();
        let trust_proxy_headers = env_bool_or("TRUST_PROXY_HEADERS", false);

        Self {
            username,
            password,
            session_secret,
            bind,
            process_timeout: Duration::from_secs(process_timeout),
            cookie_secure,
            trust_proxy_headers,
        }
    }
}

fn required_env_non_empty(key: &str) -> String {
    let value = env::var(key).unwrap_or_default();
    if value.is_empty() {
        panic!("{key} must be set");
    }
    value
}

fn env_u64_or(key: &str, default: u64) -> u64 {
    match env::var(key) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("{key} must be an integer")),
        Err(_) => default,
    }
}

fn env_bool_or(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "on" | "yes")
        }
        Err(_) => default,
    }
}

fn cookie_secure_mode_from_env() -> CookieSecureMode {
    let v = env::var("COOKIE_SECURE").unwrap_or_else(|_| "auto".to_string());
    match v.trim().to_ascii_lowercase().as_str() {
        "auto" => CookieSecureMode::Auto,
        "true" | "1" | "on" | "yes" => CookieSecureMode::Always,
        "false" | "0" | "off" | "no" => CookieSecureMode::Never,
        _ => panic!("COOKIE_SECURE must be one of: auto, true, false"),
    }
}
