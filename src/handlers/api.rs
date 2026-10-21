use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::multipart::Field;
use axum::extract::multipart::MultipartRejection;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde::Serialize;
use tempfile::TempDir;
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::io::ReaderStream;
use tower_cookies::Cookies;
use tracing::{error, info};

use crate::constants::{MAX_FILE_BYTES, MAX_PDFS};
use crate::error::AppError;
use crate::pdf::MergePageRef;
use crate::state::AppState;

const STREAM_BUF_BYTES: usize = 64 * 1024;

fn internal<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Internal(e.to_string())
}

fn bad_request<E: std::fmt::Display>(e: E) -> AppError {
    AppError::BadRequest(e.to_string())
}

const UPLOAD_BUF_BYTES: usize = 256 * 1024;
const PDF_MAGIC: &[u8] = b"%PDF-";

/// Stream a multipart PDF field to `path`, enforcing type, size and the PDF header.
async fn write_validated_pdf_field_to_path(
    field: &mut Field<'_>,
    path: &Path,
) -> Result<String, AppError> {
    let content_type = field
        .content_type()
        .map(|m| m.split(';').next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let file_name = field.file_name().unwrap_or("file.pdf").to_string();

    if !content_type.is_empty() && content_type != mime::APPLICATION_PDF.essence_str() {
        return Err(AppError::BadRequest(format!(
            "Only PDF files are allowed (got {content_type} for {file_name})"
        )));
    }

    // Buffer writes: multipart chunks are small and each tokio::fs write is a blocking-pool hop.
    let mut out = BufWriter::with_capacity(
        UPLOAD_BUF_BYTES,
        fs::File::create(path).await.map_err(internal)?,
    );
    let mut header: Vec<u8> = Vec::with_capacity(PDF_MAGIC.len());
    let mut written: usize = 0;
    while let Some(chunk) = field.chunk().await.map_err(bad_request)? {
        written = written.saturating_add(chunk.len());
        if written > MAX_FILE_BYTES {
            return Err(AppError::BadRequest(format!(
                "{file_name} is too large (max {} MB)",
                MAX_FILE_BYTES / 1024 / 1024
            )));
        }
        if header.len() < PDF_MAGIC.len() {
            let take = (PDF_MAGIC.len() - header.len()).min(chunk.len());
            header.extend_from_slice(&chunk[..take]);
            if !PDF_MAGIC.starts_with(&header) {
                break;
            }
        }
        out.write_all(&chunk).await.map_err(internal)?;
    }
    if header != PDF_MAGIC {
        return Err(AppError::BadRequest(format!(
            "{file_name} does not look like a PDF"
        )));
    }
    out.flush().await.map_err(internal)?;

    Ok(file_name)
}

async fn stream_file_response<K: Clone + Send + Sync + 'static>(
    path: &Path,
    keep_alive: K,
    apply_headers: impl FnOnce(&mut axum::http::HeaderMap) -> Result<(), AppError>,
) -> Result<Response, AppError> {
    let meta = fs::metadata(path).await.map_err(internal)?;
    let content_len = meta.len();

    let file = fs::File::open(path).await.map_err(internal)?;
    let body = Body::from_stream(ReaderStream::with_capacity(file, STREAM_BUF_BYTES));

    let mut res = Response::new(body);
    // Keep the file's directory alive for the duration of the response body stream.
    res.extensions_mut().insert(keep_alive);
    *res.status_mut() = StatusCode::OK;

    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_len.to_string()).map_err(internal)?,
    );
    apply_headers(headers)?;
    Ok(res)
}

#[derive(Serialize)]
pub(crate) struct NPagesResponse {
    pub(crate) pages: usize,
    pub(crate) upload_id: String,
}

pub(crate) async fn npages(
    State(state): State<AppState>,
    cookies: Cookies,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;

    let mut multipart = multipart.map_err(|e| {
        error!(error = %e, "multipart parse failed");
        AppError::BadRequest("Error parsing multipart/form-data request".to_string())
    })?;

    let tmp = TempDir::new().map_err(|e| AppError::Internal(e.to_string()))?;
    let mut pdf_path: Option<PathBuf> = None;
    let mut file_name: Option<String> = None;

    while let Some(mut field) = multipart.next_field().await.map_err(bad_request)? {
        if field.name() != Some("file") {
            continue;
        }

        let path = tmp.path().join("in.pdf");
        let f_name = write_validated_pdf_field_to_path(&mut field, &path).await?;
        pdf_path = Some(path);
        file_name = Some(f_name);
        break;
    }

    let Some(path) = pdf_path else {
        return Err(AppError::BadRequest("Missing file".to_string()));
    };

    let pages = crate::pdf::qpdf_show_npages_with_timeout(&path, state.process_timeout).await?;
    let upload_id = state.uploads.put(tmp, path, pages);
    info!(
        pages,
        file = %file_name.unwrap_or_else(|| "file.pdf".to_string()),
        "computed page count"
    );
    Ok(Json(NPagesResponse { pages, upload_id }).into_response())
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PageKind {
    #[default]
    Thumb,
    Full,
}

#[derive(Deserialize)]
pub(crate) struct PageQuery {
    #[serde(default)]
    pub(crate) kind: PageKind,
}

const THUMB_DPI: u32 = 40;
const FULL_DPI: u32 = 144;

pub(crate) async fn page_image(
    State(state): State<AppState>,
    cookies: Cookies,
    AxumPath((upload_id, page)): AxumPath<(String, usize)>,
    Query(query): Query<PageQuery>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;

    let Some(upload) = state.uploads.get(&upload_id) else {
        return Err(AppError::BadRequest("Unknown upload id".to_string()));
    };

    if page == 0 || page > upload.pages {
        return Err(AppError::BadRequest(format!(
            "Invalid page (must be between 1 and {})",
            upload.pages
        )));
    }

    let image_path = match query.kind {
        PageKind::Thumb => {
            // One gs run for all pages is ~20x faster than one run per page, and avoids a
            // burst of parallel processes when the client lazy-loads many thumbnails.
            let dir = upload.dir.path().join("thumbs");
            upload
                .thumbs
                .get_or_try_init(|| async {
                    fs::create_dir_all(&dir).await.map_err(internal)?;
                    crate::pdf::ghostscript_render_jpeg_with_timeout(
                        &upload.pdf_path,
                        None,
                        THUMB_DPI,
                        &dir.join("%d.jpg"),
                        state.process_timeout,
                    )
                    .await
                })
                .await?;
            dir.join(format!("{page}.jpg"))
        }
        PageKind::Full => {
            let path = upload.dir.path().join(format!("full_{page}.jpg"));
            if fs::try_exists(&path).await.map_err(internal)? {
                path
            } else {
                let tmp_path = upload
                    .dir
                    .path()
                    .join(format!("full_{page}.{}.tmp", uuid::Uuid::new_v4()));
                crate::pdf::ghostscript_render_jpeg_with_timeout(
                    &upload.pdf_path,
                    Some(page),
                    FULL_DPI,
                    &tmp_path,
                    state.process_timeout,
                )
                .await?;
                // rename() replaces atomically, so a concurrent render of the same page is harmless.
                fs::rename(&tmp_path, &path).await.map_err(internal)?;
                path
            }
        }
    };

    stream_file_response(&image_path, upload.clone(), |headers| {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=86400, immutable"),
        );
        Ok(())
    })
    .await
}

pub(crate) async fn delete_upload(
    State(state): State<AppState>,
    cookies: Cookies,
    AxumPath(upload_id): AxumPath<String>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;
    state.uploads.remove(&upload_id);
    Ok((StatusCode::NO_CONTENT, "").into_response())
}

#[derive(Deserialize)]
pub(crate) struct MergeRequest {
    #[serde(default = "default_merge_quality")]
    pub(crate) quality: u8,
    #[serde(default)]
    pub(crate) linearize: bool,
    pub(crate) layout: Vec<MergePageRef>,
    pub(crate) uploads: HashMap<String, String>,
}

fn default_merge_quality() -> u8 {
    80
}

pub(crate) async fn merge(
    State(state): State<AppState>,
    cookies: Cookies,
    payload: Result<Json<MergeRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;

    let Json(req) = payload.map_err(|e| {
        error!(error = %e, "json parse failed");
        AppError::BadRequest("Invalid JSON request body".to_string())
    })?;

    if req.layout.is_empty() {
        return Err(AppError::BadRequest("Layout is empty".to_string()));
    }

    if req.uploads.is_empty() {
        return Err(AppError::BadRequest("No uploads provided".to_string()));
    }

    if !(10..=100).contains(&req.quality) {
        return Err(AppError::BadRequest(
            "Quality must be between 10 and 100".to_string(),
        ));
    }

    let mut used_docs: HashSet<&str> = HashSet::new();
    for r in &req.layout {
        used_docs.insert(r.doc.as_str());
    }
    if used_docs.len() > MAX_PDFS {
        return Err(AppError::BadRequest(format!(
            "Too many PDFs (max {MAX_PDFS})"
        )));
    }

    let mut uploads_by_doc: HashMap<String, Arc<crate::uploads::Upload>> =
        HashMap::with_capacity(used_docs.len());
    for doc in used_docs {
        let upload_id = req
            .uploads
            .get(doc)
            .ok_or_else(|| AppError::BadRequest(format!("Missing upload id for doc: {doc}")))?;
        let Some(upload) = state.uploads.get(upload_id) else {
            return Err(AppError::BadRequest(format!(
                "Unknown or expired upload id for doc: {doc}"
            )));
        };
        uploads_by_doc.insert(doc.to_string(), upload);
    }

    for r in &req.layout {
        let Some(upload) = uploads_by_doc.get(&r.doc) else {
            return Err(AppError::BadRequest(format!(
                "Layout references unknown doc id: {}",
                r.doc
            )));
        };
        if r.page == 0 || r.page > upload.pages {
            return Err(AppError::BadRequest(format!(
                "Invalid page {} for doc {} (max {})",
                r.page, r.doc, upload.pages
            )));
        }
    }

    let inputs_by_id: HashMap<String, PathBuf> = uploads_by_doc
        .iter()
        .map(|(doc, upload)| (doc.clone(), upload.pdf_path.clone()))
        .collect();

    let tmp = Arc::new(TempDir::new().map_err(internal)?);
    let assembled = crate::pdf::qpdf_assemble_pages_with_timeout(
        tmp.as_ref(),
        &inputs_by_id,
        &req.layout,
        state.process_timeout,
    )
    .await?;
    let merged_path = crate::pdf::merge_with_ghostscript_to_file_with_timeout(
        tmp.as_ref(),
        &[assembled],
        req.quality,
        state.process_timeout,
    )
    .await?;

    let output_path = if req.linearize {
        crate::pdf::qpdf_linearize_file_with_timeout(
            tmp.as_ref(),
            &merged_path,
            state.process_timeout,
        )
        .await?
    } else {
        merged_path
    };

    stream_file_response(&output_path, tmp, |headers| {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/pdf"),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"merged.pdf\""),
        );
        Ok(())
    })
    .await
}
