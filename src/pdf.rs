use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use axum::extract::multipart::Field;
use bytes::Bytes;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::constants::MAX_FILE_BYTES;
use crate::error::AppError;

#[derive(serde::Deserialize)]
pub(crate) struct MergePageRef {
    pub(crate) doc: String,
    pub(crate) page: usize,
}

pub(crate) async fn write_multipart_field_to_file(
    field: &mut Field<'_>,
    path: &Path,
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

pub(crate) async fn looks_like_pdf(path: &Path) -> Result<bool, AppError> {
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

pub(crate) async fn qpdf_show_npages_with_timeout(
    path: &Path,
    process_timeout: Duration,
) -> Result<usize, AppError> {
    let mut cmd = Command::new("qpdf");
    cmd.arg("--show-npages").arg(path);

    let output = output_with_timeout(cmd, process_timeout, "qpdf").await?;

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

pub(crate) async fn qpdf_assemble_pages_with_timeout(
    tmp: &TempDir,
    inputs_by_id: &std::collections::HashMap<String, PathBuf>,
    layout: &[MergePageRef],
    process_timeout: Duration,
) -> Result<PathBuf, AppError> {
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

    let output = output_with_timeout(cmd, process_timeout, "qpdf").await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    Ok(output_path)
}

pub(crate) async fn qpdf_linearize_bytes_with_timeout(
    tmp: &TempDir,
    input: Bytes,
    process_timeout: Duration,
) -> Result<Bytes, AppError> {
    let in_path = tmp
        .path()
        .join(format!("lin_in_{}.pdf", uuid::Uuid::new_v4()));
    let out_path = tmp
        .path()
        .join(format!("lin_out_{}.pdf", uuid::Uuid::new_v4()));

    tokio::fs::write(&in_path, &input)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut cmd = Command::new("qpdf");
    cmd.arg("--linearize").arg(&in_path).arg(&out_path);

    let output = output_with_timeout(cmd, process_timeout, "qpdf").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    let bytes = tokio::fs::read(&out_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Bytes::from(bytes))
}

pub(crate) async fn merge_with_ghostscript_with_timeout(
    tmp: &TempDir,
    input_paths: &[PathBuf],
    quality: u8,
    process_timeout: Duration,
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

    let output = output_with_timeout(cmd, process_timeout, "ghostscript").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("ghostscript failed: {stderr}")));
    }

    let bytes = tokio::fs::read(&output_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Bytes::from(bytes))
}

fn quality_to_gs_params(quality: u8) -> (i32, i32) {
    let q = quality.clamp(10, 100) as f64;
    let t = (q - 10.0) / 90.0;
    let dpi = (72.0 + t * (300.0 - 72.0)).round() as i32;
    let jpegq = (20.0 + t * (95.0 - 20.0)).round() as i32;
    (dpi, jpegq)
}

async fn output_with_timeout(
    mut cmd: Command,
    process_timeout: Duration,
    what: &str,
) -> Result<Output, AppError> {
    cmd.kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to start {what}: {e}")))?;

    match timeout(process_timeout, child.wait_with_output()).await {
        Ok(output) => output.map_err(|e| AppError::Internal(format!("{what} failed: {e}"))),
        Err(_) => Err(AppError::Internal(format!(
            "{what} timed out after {}s",
            process_timeout.as_secs()
        ))),
    }
}
