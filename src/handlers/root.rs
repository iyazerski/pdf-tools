use axum::extract::State;
use axum::response::Html;
use tower_cookies::Cookies;

use crate::error::AppError;
use crate::pages::{render_app_page, render_login_page};
use crate::state::AppState;

pub(crate) async fn index(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Html<String>, AppError> {
    let is_authed = state.authed_username(&cookies).is_some();
    if is_authed {
        Ok(Html(render_app_page()))
    } else {
        Ok(Html(render_login_page(None)))
    }
}
