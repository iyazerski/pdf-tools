use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::multipart::Field;
use axum::extract::multipart::MultipartRejection;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Form, Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::Json;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tower_cookies::cookie::SameSite;
use tower_cookies::{Cookie, Cookies};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnResponse, TraceLayer};
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tracing::Level;
use tracing::{error, info};

type HmacSha256 = Hmac<Sha256>;

const MAX_PDFS: usize = 10;
const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
const MAX_BODY_BYTES: usize = (MAX_PDFS * MAX_FILE_BYTES) + (5 * 1024 * 1024);
const SESSION_COOKIE_NAME: &str = "pdf_tools_session";

#[derive(Clone)]
struct AppState {
    auth: Arc<AuthConfig>,
    signer: Arc<SessionSigner>,
}

#[derive(Clone)]
struct AuthConfig {
    username: String,
    password: String,
}

#[derive(Clone)]
struct SessionSigner {
    key: Vec<u8>,
    ttl: Duration,
}

#[derive(Debug)]
enum AppError {
    Unauthorized,
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            AppError::Internal(msg) => {
                error!("{msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
struct SessionPayload {
    u: String,
    exp_unix: i64,
}

#[derive(Serialize)]
struct NPagesResponse {
    pages: usize,
}

#[derive(Deserialize)]
struct MergePageRef {
    doc: String,
    page: usize,
}

impl SessionSigner {
    fn new(key: Vec<u8>, ttl: Duration) -> Self {
        Self { key, ttl }
    }

    fn issue(&self, username: &str, now: OffsetDateTime) -> String {
        let payload = SessionPayload {
            u: username.to_string(),
            exp_unix: (now + self.ttl).unix_timestamp(),
        };

        let payload_json =
            serde_json::to_vec(&payload).expect("session payload must serialize to JSON");
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);

        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC key must be non-empty");
        mac.update(payload_b64.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);

        format!("v1.{payload_b64}.{sig_b64}")
    }

    fn verify(&self, token: &str, now: OffsetDateTime) -> Option<SessionPayload> {
        let mut parts = token.split('.');
        let version = parts.next()?;
        let payload_b64 = parts.next()?;
        let sig_b64 = parts.next()?;
        if version != "v1" || parts.next().is_some() {
            return None;
        }

        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig_b64.as_bytes())
            .ok()?;

        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&sig).ok()?;

        let payload_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64.as_bytes())
            .ok()?;
        let payload: SessionPayload = serde_json::from_slice(&payload_json).ok()?;
        if payload.exp_unix <= now.unix_timestamp() {
            return None;
        }
        Some(payload)
    }
}

fn authed_username(state: &AppState, cookies: &Cookies) -> Option<String> {
    let token = cookies.get(SESSION_COOKIE_NAME)?;
    let now = OffsetDateTime::now_utc();
    state.signer.verify(token.value(), now).map(|p| p.u)
}

fn require_auth(state: &AppState, cookies: &Cookies) -> Result<String, AppError> {
    authed_username(state, cookies).ok_or(AppError::Unauthorized)
}

async fn index(State(state): State<AppState>, cookies: Cookies) -> Result<Html<String>, AppError> {
    let is_authed = authed_username(&state, &cookies).is_some();
    if is_authed {
        Ok(Html(render_app_page()))
    } else {
        Ok(Html(render_login_page(None)))
    }
}

async fn login(
    State(state): State<AppState>,
    cookies: Cookies,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    if form.username != state.auth.username || form.password != state.auth.password {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Html(render_login_page(Some("Invalid username or password."))),
        )
            .into_response());
    }

    let token = state
        .signer
        .issue(&form.username, OffsetDateTime::now_utc());
    let mut cookie = Cookie::new(SESSION_COOKIE_NAME, token);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookies.add(cookie);

    Ok(Redirect::to("/").into_response())
}

async fn logout(State(state): State<AppState>, cookies: Cookies) -> Result<Response, AppError> {
    let _ = authed_username(&state, &cookies);
    let mut cookie = Cookie::new(SESSION_COOKIE_NAME, "");
    cookie.set_path("/");
    cookie.make_removal();
    cookies.add(cookie);
    Ok(Redirect::to("/").into_response())
}

async fn npages(
    State(state): State<AppState>,
    cookies: Cookies,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let _username = require_auth(&state, &cookies)?;

    let mut multipart = multipart.map_err(|e| {
        error!(error = %e, "multipart parse failed");
        AppError::BadRequest("Error parsing multipart/form-data request".to_string())
    })?;

    let tmp = TempDir::new().map_err(|e| AppError::Internal(e.to_string()))?;
    let mut pdf_path: Option<std::path::PathBuf> = None;
    let mut file_name: Option<String> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            continue;
        }

        let content_type = field
            .content_type()
            .map(|m| m.split(';').next().unwrap_or("").trim().to_string())
            .unwrap_or_default();
        let f_name = field.file_name().unwrap_or("file.pdf").to_string();

        if !content_type.is_empty() && content_type != mime::APPLICATION_PDF.essence_str() {
            return Err(AppError::BadRequest(format!(
                "Only PDF files are allowed (got {content_type} for {f_name})"
            )));
        }

        let path = tmp.path().join("in.pdf");
        let written = write_multipart_field_to_file(&mut field, &path).await?;
        if written > MAX_FILE_BYTES {
            return Err(AppError::BadRequest(format!(
                "{f_name} is too large (max {} MB)",
                MAX_FILE_BYTES / 1024 / 1024
            )));
        }
        if !looks_like_pdf(&path).await? {
            return Err(AppError::BadRequest(format!(
                "{f_name} does not look like a PDF"
            )));
        }

        pdf_path = Some(path);
        file_name = Some(f_name);
        break;
    }

    let Some(path) = pdf_path else {
        return Err(AppError::BadRequest("Missing file".to_string()));
    };

    let pages = qpdf_show_npages(&path).await?;
    info!(
        pages,
        file = %file_name.unwrap_or_else(|| "file.pdf".to_string()),
        "computed page count"
    );
    Ok(Json(NPagesResponse { pages }).into_response())
}

async fn merge(
    State(state): State<AppState>,
    cookies: Cookies,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let _username = require_auth(&state, &cookies)?;

    let mut multipart = multipart.map_err(|e| {
        error!(error = %e, "multipart parse failed");
        AppError::BadRequest("Error parsing multipart/form-data request".to_string())
    })?;

    let mut quality: u8 = 80;
    let mut layout_json: Option<String> = None;
    let tmp = TempDir::new().map_err(|e| AppError::Internal(e.to_string()))?;
    let mut input_paths_legacy: Vec<std::path::PathBuf> = Vec::new();
    let mut inputs_by_id: std::collections::HashMap<String, std::path::PathBuf> =
        std::collections::HashMap::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "quality" {
            let value = field
                .text()
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            quality = value
                .trim()
                .parse::<u8>()
                .map_err(|_| AppError::BadRequest("Invalid quality".to_string()))?;
            continue;
        }
        if name == "layout" {
            layout_json = Some(
                field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?,
            );
            continue;
        }

        let content_type = field
            .content_type()
            .map(|m| m.split(';').next().unwrap_or("").trim().to_string())
            .unwrap_or_default();
        let file_name = field.file_name().unwrap_or("file.pdf").to_string();

        // Why: browsers can send empty content-type; we still validate by magic header after write.
        if !content_type.is_empty() && content_type != mime::APPLICATION_PDF.essence_str() {
            return Err(AppError::BadRequest(format!(
                "Only PDF files are allowed (got {content_type} for {file_name})"
            )));
        }

        let (doc_id, legacy_idx) = if let Some(rest) = name.strip_prefix("file_") {
            (rest.to_string(), None)
        } else if name == "files" {
            (format!("legacy_{}", input_paths_legacy.len()), Some(input_paths_legacy.len()))
        } else {
            return Err(AppError::BadRequest(format!(
                "Unexpected form field: {name}"
            )));
        };

        if legacy_idx.is_some() && input_paths_legacy.len() >= MAX_PDFS {
            return Err(AppError::BadRequest(format!(
                "Too many PDFs (max {MAX_PDFS})"
            )));
        }
        if inputs_by_id.len() >= MAX_PDFS && legacy_idx.is_none() {
            return Err(AppError::BadRequest(format!(
                "Too many PDFs (max {MAX_PDFS})"
            )));
        }

        let path = tmp.path().join(format!("in_{}.pdf", uuid::Uuid::new_v4()));
        let written = write_multipart_field_to_file(&mut field, &path).await?;
        if written > MAX_FILE_BYTES {
            return Err(AppError::BadRequest(format!(
                "{file_name} is too large (max {} MB)",
                MAX_FILE_BYTES / 1024 / 1024
            )));
        }

        if !looks_like_pdf(&path).await? {
            return Err(AppError::BadRequest(format!(
                "{file_name} does not look like a PDF"
            )));
        }

        if legacy_idx.is_some() {
            input_paths_legacy.push(path);
        } else {
            if inputs_by_id.insert(doc_id.clone(), path).is_some() {
                return Err(AppError::BadRequest(format!(
                    "Duplicate document id: {doc_id}"
                )));
            }
        }
    }

    if input_paths_legacy.is_empty() && inputs_by_id.is_empty() {
        return Err(AppError::BadRequest("No PDF files uploaded".to_string()));
    }

    if !(10..=100).contains(&quality) {
        return Err(AppError::BadRequest(
            "Quality must be between 10 and 100".to_string(),
        ));
    }

    let output_bytes = if let Some(layout_json) = layout_json {
        let layout: Vec<MergePageRef> = serde_json::from_str(&layout_json)
            .map_err(|_| AppError::BadRequest("Invalid layout".to_string()))?;
        if layout.is_empty() {
            return Err(AppError::BadRequest("Layout is empty".to_string()));
        }
        if inputs_by_id.is_empty() {
            return Err(AppError::BadRequest(
                "Layout provided but no file_* parts found".to_string(),
            ));
        }

        let mut pages_by_doc: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (doc, path) in &inputs_by_id {
            let pages = qpdf_show_npages(path).await?;
            pages_by_doc.insert(doc.clone(), pages);
        }

        for r in &layout {
            let Some(max_pages) = pages_by_doc.get(&r.doc) else {
                return Err(AppError::BadRequest(format!(
                    "Layout references unknown doc id: {}",
                    r.doc
                )));
            };
            if r.page == 0 || r.page > *max_pages {
                return Err(AppError::BadRequest(format!(
                    "Invalid page {} for doc {} (max {})",
                    r.page, r.doc, max_pages
                )));
            }
        }

        let assembled = qpdf_assemble_pages(&tmp, &inputs_by_id, &layout).await?;
        merge_with_ghostscript(&tmp, &[assembled], quality).await?
    } else {
        merge_with_ghostscript(&tmp, &input_paths_legacy, quality).await?
    };

    let mut res = Response::new(Body::from(output_bytes));
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"merged.pdf\""),
    );
    Ok(res)
}

async fn qpdf_assemble_pages(
    tmp: &TempDir,
    inputs_by_id: &std::collections::HashMap<String, std::path::PathBuf>,
    layout: &[MergePageRef],
) -> Result<std::path::PathBuf, AppError> {
    let output_path = tmp
        .path()
        .join(format!("assembled_{}.pdf", uuid::Uuid::new_v4()));

    let mut cmd = Command::new("qpdf");
    cmd.arg("--empty").arg("--pages");
    for r in layout {
        let path = inputs_by_id
            .get(&r.doc)
            .ok_or_else(|| AppError::BadRequest(format!("Unknown doc id: {}", r.doc)))?;
        cmd.arg(path).arg(r.page.to_string());
    }
    cmd.arg("--").arg(&output_path);

    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to start qpdf: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    Ok(output_path)
}

async fn write_multipart_field_to_file(
    field: &mut Field<'_>,
    path: &std::path::Path,
) -> Result<usize, AppError> {
    let mut out = tokio::fs::File::create(path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut written: usize = 0;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        written = written.saturating_add(chunk.len());
        if written > MAX_FILE_BYTES {
            return Ok(written);
        }
        out.write_all(&chunk)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }
    out.flush()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(written)
}

async fn looks_like_pdf(path: &std::path::Path) -> Result<bool, AppError> {
    let mut f = tokio::fs::File::open(path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut buf = [0u8; 5];
    let n = f
        .read(&mut buf)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(n == 5 && &buf == b"%PDF-")
}

async fn merge_with_ghostscript(
    tmp: &TempDir,
    input_paths: &[std::path::PathBuf],
    quality: u8,
) -> Result<Bytes, AppError> {
    let output_path = tmp.path().join(format!("out_{}.pdf", uuid::Uuid::new_v4()));
    let (dpi, jpegq) = quality_to_gs_params(quality);

    let mut cmd = Command::new("gs");
    cmd.arg("-q")
        .arg("-dNOPAUSE")
        .arg("-dBATCH")
        .arg("-sDEVICE=pdfwrite")
        .arg("-dCompatibilityLevel=1.4")
        .arg("-dDetectDuplicateImages=true")
        .arg("-dCompressFonts=true")
        .arg("-dSubsetFonts=true")
        .arg("-dDownsampleColorImages=true")
        .arg("-dDownsampleGrayImages=true")
        .arg("-dDownsampleMonoImages=true")
        .arg("-dColorImageDownsampleType=/Bicubic")
        .arg("-dGrayImageDownsampleType=/Bicubic")
        .arg(format!("-dColorImageResolution={dpi}"))
        .arg(format!("-dGrayImageResolution={dpi}"))
        .arg("-dMonoImageResolution=600")
        .arg(format!("-dJPEGQ={jpegq}"))
        .arg(format!("-sOutputFile={}", output_path.to_string_lossy()));

    for p in input_paths {
        cmd.arg(p);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to start ghostscript: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("ghostscript failed: {stderr}")));
    }

    let bytes = tokio::fs::read(&output_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Bytes::from(bytes))
}

async fn qpdf_show_npages(path: &std::path::Path) -> Result<usize, AppError> {
    let output = Command::new("qpdf")
        .arg("--show-npages")
        .arg(path)
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to start qpdf: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pages = stdout
        .trim()
        .parse::<usize>()
        .map_err(|_| AppError::Internal("Failed to parse qpdf output".to_string()))?;
    Ok(pages)
}

fn quality_to_gs_params(quality: u8) -> (i32, i32) {
    // Why: keep 10..100 slider continuous while staying in reasonable gs ranges.
    let q = quality.clamp(10, 100) as f64;
    let t = (q - 10.0) / 90.0;
    let dpi = (72.0 + t * (300.0 - 72.0)).round() as i32;
    let jpegq = (20.0 + t * (95.0 - 20.0)).round() as i32;
    (dpi, jpegq)
}

fn render_login_page(error: Option<&str>) -> String {
    let css = include_str!("../static/styles.css");
    let err_html = error
        .map(|m| {
            format!(
                r#"<div class="alert" role="alert">{}</div>"#,
                html_escape(m)
            )
        })
        .unwrap_or_default();

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>PDF Tools — Sign in</title>
    <style>{css}</style>
  </head>
  <body>
    <div class="bg"></div>
    <main class="shell">
      <section class="card">
        <div class="brand">
          <div class="logo" aria-hidden="true">PDF</div>
          <div>
            <div class="title">PDF Tools</div>
            <div class="subtitle">Merge PDFs and optimize size</div>
          </div>
        </div>
        {err_html}
        <form class="form" method="post" action="/login" autocomplete="off">
          <label class="label">Username</label>
          <input class="input" name="username" required />
          <label class="label">Password</label>
          <input class="input" type="password" name="password" required />
          <button class="btn primary" type="submit">Sign in</button>
        </form>
        <div class="hint">Credentials are configured via environment variables.</div>
      </section>
    </main>
  </body>
</html>"#
    )
}

fn render_app_page() -> String {
    let css = include_str!("../static/styles.css");
    let js = include_str!("../static/app.js");

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>PDF Tools</title>
    <style>{css}</style>
  </head>
  <body>
    <div class="bg"></div>
    <main class="shell">
      <header class="topbar">
        <div class="brand">
          <div class="logo" aria-hidden="true">PDF</div>
          <div>
            <div class="title">PDF Tools</div>
            <div class="subtitle">Merge PDFs in order, pick output quality</div>
          </div>
        </div>
        <form method="post" action="/logout">
          <button class="btn ghost" type="submit">Log out</button>
        </form>
      </header>

      <section class="grid">
        <div class="card">
          <div class="section-title">1) Upload PDFs</div>
          <div id="dropzone" class="dropzone" tabindex="0" role="button" aria-label="Upload PDFs">
            <div class="dz-title">Drag & drop up to 10 PDF files</div>
            <div class="dz-sub">…or click to choose files</div>
          </div>
          <input id="fileInput" type="file" accept="application/pdf,.pdf" multiple hidden />

          <div class="list-head">
            <div class="muted">Order matters (drag to reorder)</div>
            <div class="muted"><span id="count">0</span>/10</div>
          </div>
          <ul id="fileList" class="file-list" aria-label="Uploaded PDFs"></ul>
          <div id="empty" class="empty">No files yet.</div>
        </div>

        <div class="card">
          <div class="section-title">2) Output settings</div>
          <div class="row">
            <div>
              <div class="label-row">
                <div class="label">Quality</div>
                <div class="pill"><span id="qualityValue">80</span>%</div>
              </div>
              <input id="quality" class="range" type="range" min="10" max="100" value="80" />
            </div>
          </div>

          <div class="stats">
            <div class="stat">
              <div class="stat-k">Input size</div>
              <div class="stat-v" id="inputSize">0 B</div>
            </div>
            <div class="stat">
              <div class="stat-k">Estimated output</div>
              <div class="stat-v" id="estimatedSize">0 B</div>
            </div>
          </div>

          <div class="actions">
            <button id="mergeBtn" class="btn primary" type="button" disabled>Merge</button>
            <button id="clearBtn" class="btn" type="button" disabled>Clear</button>
          </div>
          <div class="hint">Nothing is stored server-side; refresh clears the workspace.</div>

          <div id="toast" class="toast" role="status" aria-live="polite"></div>
        </div>
      </section>
    </main>

    <script>{js}</script>
  </body>
</html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Why: local runs should work with a `.env` file without requiring shell export/source.
    let _ = dotenvy::dotenv();

    let username = env::var("APP_USERNAME").unwrap_or_default();
    let password = env::var("APP_PASSWORD").unwrap_or_default();
    if username.is_empty() || password.is_empty() {
        panic!("APP_USERNAME and APP_PASSWORD must be set");
    }

    let session_secret = env::var("SESSION_SECRET").unwrap_or_default();
    if session_secret.is_empty() {
        panic!("SESSION_SECRET must be set");
    }

    let bind = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let state = AppState {
        auth: Arc::new(AuthConfig { username, password }),
        signer: Arc::new(SessionSigner::new(
            session_secret.into_bytes(),
            Duration::hours(24),
        )),
    };

    let global_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(10)
                .burst_size(30)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let auth_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(1)
                .burst_size(5)
                .finish()
                .expect("governor config must build"),
        ),
    };
    let api_governor = GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(2)
                .burst_size(10)
                .finish()
                .expect("governor config must build"),
        ),
    };

    let auth_routes = Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route_layer(auth_governor);

    let api_routes = Router::new()
        .route("/merge", post(merge))
        .route("/npages", post(npages))
        .route_layer(api_governor);

    let app = Router::new()
        .route("/", get(index))
        .merge(auth_routes)
        .nest("/api", api_routes)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .with_state(state)
        .layer(tower_cookies::CookieManagerLayer::new())
        .layer(global_governor)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        );

    info!(bind = %bind, "starting server");
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .expect("bind must succeed");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server must start");
    info!("server stopped");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown signal received");
}
