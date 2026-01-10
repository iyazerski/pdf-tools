use std::fs;
use std::sync::OnceLock;

use crate::error::AppError;

const INDEX_HTML_PATH: &str = "templates/index.html";
const AUTH_DATA_PLACEHOLDER: &str = "{{AUTH_DATA}}";
const LOGIN_ERROR_DATA_PLACEHOLDER: &str = "{{LOGIN_ERROR_DATA}}";

static INDEX_HTML_TEMPLATE: OnceLock<String> = OnceLock::new();

fn load_index_template() -> Result<&'static str, AppError> {
    if let Some(template) = INDEX_HTML_TEMPLATE.get() {
        return Ok(template.as_str());
    }

    let template = fs::read_to_string(INDEX_HTML_PATH)
        .map_err(|e| AppError::Internal(format!("Failed to read {INDEX_HTML_PATH}: {e}")))?;

    let _ = INDEX_HTML_TEMPLATE.set(template);
    Ok(INDEX_HTML_TEMPLATE
        .get()
        .expect("OnceLock must be initialized")
        .as_str())
}

pub(crate) fn render_app_page(is_authed: bool, show_login_error: bool) -> Result<String, AppError> {
    let auth_data = if is_authed { "1" } else { "0" };
    let login_error_data = if show_login_error { "1" } else { "0" };

    let template = load_index_template()?;

    // Fail fast on template drift: it's easy to remove placeholders by accident.
    if !template.contains(AUTH_DATA_PLACEHOLDER) || !template.contains(LOGIN_ERROR_DATA_PLACEHOLDER)
    {
        return Err(AppError::Internal(format!(
            "static template {INDEX_HTML_PATH} is missing required placeholders"
        )));
    }

    Ok(template
        .replace(AUTH_DATA_PLACEHOLDER, auth_data)
        .replace(LOGIN_ERROR_DATA_PLACEHOLDER, login_error_data))
}
