use std::collections::HashMap;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use bytes::Bytes;
use serde::Deserialize;
use serde::Serialize;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_cookies::Cookies;
use tracing::{error, info};

use crate::constants::{MAX_FILE_BYTES, MAX_PDFS};
use crate::error::AppError;
use crate::pdf::{looks_like_pdf, write_multipart_field_to_file, MergePageRef};
use crate::state::AppState;
use crate::util::parse_bool_loose;

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

    let pages = crate::pdf::qpdf_show_npages_with_timeout(&path, state.process_timeout).await?;
    let upload_id = state.uploads.put_pdf(tmp, path, pages).await;
    info!(
        pages,
        file = %file_name.unwrap_or_else(|| "file.pdf".to_string()),
        "computed page count"
    );
    Ok(Json(NPagesResponse { pages, upload_id }).into_response())
}

#[derive(Deserialize)]
pub(crate) struct PagePngQuery {
    pub(crate) kind: Option<String>,
}

pub(crate) async fn page_png(
    State(state): State<AppState>,
    cookies: Cookies,
    Path((upload_id, page)): Path<(String, usize)>,
    Query(query): Query<PagePngQuery>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;

    let Some(upload) = state.uploads.get(&upload_id).await else {
        return Err(AppError::BadRequest("Unknown upload id".to_string()));
    };

    if page == 0 || page > upload.pages {
        return Err(AppError::BadRequest(format!(
            "Invalid page (must be between 1 and {})",
            upload.pages
        )));
    }

    let dpi = match query.kind.as_deref() {
        Some("thumb") => 40,
        Some("full") => 144,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "Invalid kind (expected thumb|full, got {other})"
            )));
        }
        None => 40,
    };

    let tmp = TempDir::new().map_err(|e| AppError::Internal(e.to_string()))?;
    let png_path = crate::pdf::ghostscript_render_page_png_with_timeout(
        &tmp,
        &upload.pdf_path,
        page,
        dpi,
        state.process_timeout,
    )
    .await?;

    let bytes = tokio::fs::read(png_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut res = Response::new(Body::from(Bytes::from(bytes)));
    *res.status_mut() = StatusCode::OK;
    res.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    Ok(res)
}

pub(crate) async fn delete_upload(
    State(state): State<AppState>,
    cookies: Cookies,
    Path(upload_id): Path<String>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;
    let _ = state.uploads.remove(&upload_id).await;
    Ok((StatusCode::NO_CONTENT, "").into_response())
}

pub(crate) async fn merge(
    State(state): State<AppState>,
    cookies: Cookies,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Response, AppError> {
    let _username = state.require_auth(&cookies)?;

    let mut multipart = multipart.map_err(|e| {
        error!(error = %e, "multipart parse failed");
        AppError::BadRequest("Error parsing multipart/form-data request".to_string())
    })?;

    let mut quality: u8 = 80;
    let mut linearize: bool = false;
    let mut layout_json: Option<String> = None;
    let tmp = TempDir::new().map_err(|e| AppError::Internal(e.to_string()))?;
    let mut input_paths_legacy: Vec<PathBuf> = Vec::new();
    let mut inputs_by_id: HashMap<String, PathBuf> = HashMap::new();

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
        if name == "linearize" {
            let value = field
                .text()
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            linearize = parse_bool_loose(&value);
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

        if !content_type.is_empty() && content_type != mime::APPLICATION_PDF.essence_str() {
            return Err(AppError::BadRequest(format!(
                "Only PDF files are allowed (got {content_type} for {file_name})"
            )));
        }

        let (doc_id, legacy_idx) = if let Some(rest) = name.strip_prefix("file_") {
            (rest.to_string(), None)
        } else if name == "files" {
            (
                format!("legacy_{}", input_paths_legacy.len()),
                Some(input_paths_legacy.len()),
            )
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
        } else if inputs_by_id.insert(doc_id.clone(), path).is_some() {
            return Err(AppError::BadRequest(format!(
                "Duplicate document id: {doc_id}"
            )));
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

    let output_path = if let Some(layout_json) = layout_json {
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

        let mut pages_by_doc: HashMap<String, usize> = HashMap::new();
        for (doc, path) in &inputs_by_id {
            let pages =
                crate::pdf::qpdf_show_npages_with_timeout(path, state.process_timeout).await?;
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

        let assembled = crate::pdf::qpdf_assemble_pages_with_timeout(
            &tmp,
            &inputs_by_id,
            &layout,
            state.process_timeout,
        )
        .await?;
        let merged_path = crate::pdf::merge_with_ghostscript_to_file_with_timeout(
            &tmp,
            &[assembled],
            quality,
            state.process_timeout,
        )
        .await?;
        if linearize {
            crate::pdf::qpdf_linearize_file_with_timeout(&tmp, &merged_path, state.process_timeout)
                .await?
        } else {
            merged_path
        }
    } else {
        let merged_path = crate::pdf::merge_with_ghostscript_to_file_with_timeout(
            &tmp,
            &input_paths_legacy,
            quality,
            state.process_timeout,
        )
        .await?;
        if linearize {
            crate::pdf::qpdf_linearize_file_with_timeout(&tmp, &merged_path, state.process_timeout)
                .await?
        } else {
            merged_path
        }
    };

    let meta = tokio::fs::metadata(&output_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let content_len = meta.len();

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let _tmp = tmp;
        let mut file = match tokio::fs::File::open(&output_path).await {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };

        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match file.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            if n == 0 {
                return;
            }
            if tx
                .send(Ok(Bytes::copy_from_slice(&buf[..n])))
                .await
                .is_err()
            {
                return;
            }
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));
    let mut res = Response::new(body);
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    res.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"merged.pdf\""),
    );
    res.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_len.to_string())
            .map_err(|e| AppError::Internal(e.to_string()))?,
    );
    Ok(res)
}
