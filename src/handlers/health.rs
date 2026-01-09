use axum::http::StatusCode;

pub(crate) async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}
