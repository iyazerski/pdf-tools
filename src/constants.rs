pub(crate) const MAX_PDFS: usize = 10;
pub(crate) const MAX_FILE_BYTES: usize = 30 * 1024 * 1024;
pub(crate) const MAX_BODY_BYTES: usize = (MAX_PDFS * MAX_FILE_BYTES) + (5 * 1024 * 1024);

pub(crate) const SESSION_COOKIE_NAME: &str = "pdf_tools_session";

pub(crate) const GLOBAL_RATE_LIMIT_RPS: u64 = 20;
pub(crate) const GLOBAL_RATE_LIMIT_BURST: u32 = 40;
