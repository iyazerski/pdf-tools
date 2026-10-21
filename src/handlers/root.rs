use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;
use tower_cookies::Cookies;

use crate::assets::index_page;
use crate::state::AppState;

#[derive(Deserialize)]
pub(crate) struct IndexQuery {
    pub(crate) login_error: Option<String>,
}

pub(crate) async fn index(
    State(state): State<AppState>,
    cookies: Cookies,
    Query(query): Query<IndexQuery>,
) -> Response {
    let is_authed = state.authed_username(&cookies).is_some();
    index_page(is_authed, !is_authed && query.login_error.is_some())
}
