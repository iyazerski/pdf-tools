use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;
use tower_cookies::Cookies;

use crate::error::AppError;
use crate::pages::render_app_page;
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct IndexQuery {
    pub(crate) login_error: Option<String>,
}

pub(crate) async fn index(
    State(state): State<AppState>,
    cookies: Cookies,
    Query(query): Query<IndexQuery>,
) -> Result<Html<String>, AppError> {
    let is_authed = state.authed_username(&cookies).is_some();
    Ok(Html(render_app_page(
        is_authed,
        !is_authed && query.login_error.is_some(),
    )))
}
