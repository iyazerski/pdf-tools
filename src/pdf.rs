use std::path::{Path, PathBuf};
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;

use axum::extract::multipart::Field;
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

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let pages = stdout
        .split_whitespace()
        .find_map(|t| t.parse::<usize>().ok())
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            AppError::Internal(format!(
                "Failed to parse qpdf output (stdout={}, stderr={})",
                truncate_for_log(&stdout),
                truncate_for_log(&stderr)
            ))
        })?;
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

    let runs = compress_layout_runs(layout);

    let mut cmd = Command::new("qpdf");
    cmd.arg("--empty").arg("--pages");
    for run in runs {
        let path = inputs_by_id
            .get(&run.doc)
            .ok_or_else(|| AppError::BadRequest(format!("Unknown doc id: {}", run.doc)))?;
        cmd.arg(path).arg(run.spec());
    }
    cmd.arg("--").arg(&output_path);

    let output = output_with_timeout(cmd, process_timeout, "qpdf").await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    Ok(output_path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayoutRun {
    doc: String,
    start: usize,
    end: usize,
}

impl LayoutRun {
    fn spec(&self) -> String {
        if self.start == self.end {
            self.start.to_string()
        } else {
            format!("{}-{}", self.start, self.end)
        }
    }
}

fn compress_layout_runs(layout: &[MergePageRef]) -> Vec<LayoutRun> {
    let mut runs: Vec<LayoutRun> = Vec::new();
    for r in layout {
        if let Some(last) = runs.last_mut() {
            if last.doc == r.doc && r.page == last.end.saturating_add(1) {
                last.end = r.page;
                continue;
            }
        }

        runs.push(LayoutRun {
            doc: r.doc.clone(),
            start: r.page,
            end: r.page,
        });
    }
    runs
}

fn quality_to_gs_params(quality: u8) -> (i32, i32) {
    let q = quality.clamp(10, 100) as f64;
    let t = (q - 10.0) / 90.0;
    let dpi = (72.0 + t * (300.0 - 72.0)).round() as i32;
    let jpegq = (20.0 + t * (95.0 - 20.0)).round() as i32;
    (dpi, jpegq)
}

pub(crate) async fn qpdf_linearize_file_with_timeout(
    tmp: &TempDir,
    input_path: &Path,
    process_timeout: Duration,
) -> Result<PathBuf, AppError> {
    let out_path = tmp
        .path()
        .join(format!("lin_out_{}.pdf", uuid::Uuid::new_v4()));

    let mut cmd = Command::new("qpdf");
    cmd.arg("--linearize").arg(input_path).arg(&out_path);

    let output = output_with_timeout(cmd, process_timeout, "qpdf").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("qpdf failed: {stderr}")));
    }

    Ok(out_path)
}

pub(crate) async fn merge_with_ghostscript_to_file_with_timeout(
    tmp: &TempDir,
    input_paths: &[PathBuf],
    quality: u8,
    process_timeout: Duration,
) -> Result<PathBuf, AppError> {
    let output_path = tmp.path().join(format!("out_{}.pdf", uuid::Uuid::new_v4()));
    let (dpi, jpegq) = quality_to_gs_params(quality);

    let mut cmd = Command::new("gs");
    cmd.arg("-q")
        .arg("-dSAFER")
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

    Ok(output_path)
}

pub(crate) async fn ghostscript_render_page_png_to_path_with_timeout(
    input_path: &Path,
    page: usize,
    dpi: i32,
    output_path: &Path,
    process_timeout: Duration,
) -> Result<(), AppError> {
    let mut cmd = Command::new("gs");
    cmd.arg("-q")
        .arg("-dSAFER")
        .arg("-dNOPAUSE")
        .arg("-dBATCH")
        // `pngalpha` yields transparent backgrounds; for many "programmatically generated" PDFs that
        // don't paint a page background, this makes the preview inherit our dark UI background.
        // Render an opaque image with a white background to match typical PDF viewers.
        .arg("-dBackgroundColor=16#FFFFFF")
        .arg("-sDEVICE=png16m")
        .arg("-dTextAlphaBits=4")
        .arg("-dGraphicsAlphaBits=4")
        .arg(format!("-dFirstPage={page}"))
        .arg(format!("-dLastPage={page}"))
        .arg(format!("-r{dpi}"))
        .arg(format!("-sOutputFile={}", output_path.to_string_lossy()))
        .arg(input_path);

    let output = output_with_timeout(cmd, process_timeout, "ghostscript").await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(AppError::Internal(format!("ghostscript failed: {stderr}")));
    }

    Ok(())
}

async fn output_with_timeout(
    mut cmd: Command,
    process_timeout: Duration,
    what: &str,
) -> Result<Output, AppError> {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
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

fn truncate_for_log(s: &str) -> String {
    const MAX: usize = 512;
    if s.len() <= MAX {
        return s.to_string();
    }

    let mut end = 0usize;
    for (idx, _) in s.char_indices() {
        if idx > MAX {
            break;
        }
        end = idx;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::{compress_layout_runs, MergePageRef};

    #[test]
    fn compress_layout_runs_makes_ranges_for_consecutive_pages() {
        let layout = vec![
            MergePageRef {
                doc: "a".to_string(),
                page: 1,
            },
            MergePageRef {
                doc: "a".to_string(),
                page: 2,
            },
            MergePageRef {
                doc: "a".to_string(),
                page: 3,
            },
            MergePageRef {
                doc: "b".to_string(),
                page: 10,
            },
            MergePageRef {
                doc: "b".to_string(),
                page: 11,
            },
            MergePageRef {
                doc: "a".to_string(),
                page: 4,
            },
        ];

        let runs = compress_layout_runs(&layout);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].doc, "a");
        assert_eq!(runs[0].spec(), "1-3");
        assert_eq!(runs[1].doc, "b");
        assert_eq!(runs[1].spec(), "10-11");
        assert_eq!(runs[2].doc, "a");
        assert_eq!(runs[2].spec(), "4");
    }

    #[test]
    fn compress_layout_runs_does_not_merge_non_consecutive_pages() {
        let layout = vec![
            MergePageRef {
                doc: "a".to_string(),
                page: 1,
            },
            MergePageRef {
                doc: "a".to_string(),
                page: 3,
            },
        ];
        let runs = compress_layout_runs(&layout);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].spec(), "1");
        assert_eq!(runs[1].spec(), "3");
    }
}
