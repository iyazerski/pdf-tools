pub(crate) const MAX_PDFS: usize = 10;
pub(crate) const MAX_FILE_BYTES: usize = 30 * 1024 * 1024;
pub(crate) const MAX_BODY_BYTES: usize = MAX_FILE_BYTES + (5 * 1024 * 1024);

pub(crate) const SESSION_COOKIE_NAME: &str = "pdf_tools_session";

// Keeps worst-case disk use (entries x MAX_FILE_BYTES plus one merge) under the 2Gi emptyDir.
pub(crate) const UPLOAD_CACHE_MAX_ENTRIES: usize = 15;

// Per client IP: 5 attempts at once, then one more every 2 seconds.
pub(crate) const LOGIN_RATE_LIMIT_PERIOD: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const LOGIN_RATE_LIMIT_BURST: u32 = 5;
