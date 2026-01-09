use std::env;

pub(crate) struct AppConfig {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) session_secret: String,
    pub(crate) bind: String,
}

impl AppConfig {
    pub(crate) fn from_env() -> Self {
        let username = required_env_non_empty("APP_USERNAME");
        let password = required_env_non_empty("APP_PASSWORD");
        let session_secret = required_env_non_empty("SESSION_SECRET");
        let bind = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        Self {
            username,
            password,
            session_secret,
            bind,
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
