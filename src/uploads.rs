use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::TempDir;

#[derive(Clone)]
pub(crate) struct UploadHandle {
    pub(crate) _dir_guard: Arc<TempDir>,
    pub(crate) pdf_path: PathBuf,
    pub(crate) pages: usize,
}

struct UploadEntry {
    dir: Arc<TempDir>,
    pdf_path: PathBuf,
    pages: usize,
    last_access: Instant,
}

pub(crate) struct UploadStore {
    ttl: Duration,
    max_entries: usize,
    inner: tokio::sync::Mutex<HashMap<String, UploadEntry>>,
}

impl UploadStore {
    pub(crate) fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            inner: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn put_pdf(&self, dir: TempDir, pdf_path: PathBuf, pages: usize) -> String {
        let now = Instant::now();
        let mut guard = self.inner.lock().await;
        prune_locked(&mut guard, now, self.ttl);

        // Prefer bounded memory/disk usage over preserving old uploads; this cache is an optimization.
        if guard.len() >= self.max_entries {
            evict_oldest_locked(&mut guard, self.max_entries.saturating_sub(1));
        }

        let upload_id = uuid::Uuid::new_v4().to_string();
        guard.insert(
            upload_id.clone(),
            UploadEntry {
                dir: Arc::new(dir),
                pdf_path,
                pages,
                last_access: now,
            },
        );
        upload_id
    }

    pub(crate) async fn get(&self, upload_id: &str) -> Option<UploadHandle> {
        let now = Instant::now();
        let mut guard = self.inner.lock().await;
        prune_locked(&mut guard, now, self.ttl);

        let entry = guard.get_mut(upload_id)?;
        entry.last_access = now;
        Some(UploadHandle {
            _dir_guard: entry.dir.clone(),
            pdf_path: entry.pdf_path.clone(),
            pages: entry.pages,
        })
    }

    pub(crate) async fn remove(&self, upload_id: &str) -> bool {
        let now = Instant::now();
        let mut guard = self.inner.lock().await;
        prune_locked(&mut guard, now, self.ttl);
        guard.remove(upload_id).is_some()
    }
}

fn prune_locked(map: &mut HashMap<String, UploadEntry>, now: Instant, ttl: Duration) {
    if ttl == Duration::ZERO {
        map.clear();
        return;
    }

    map.retain(|_, v| now.duration_since(v.last_access) <= ttl);
}

fn evict_oldest_locked(map: &mut HashMap<String, UploadEntry>, keep_at_most: usize) {
    if map.len() <= keep_at_most {
        return;
    }

    let mut entries: Vec<(String, Instant)> = map
        .iter()
        .map(|(k, v)| (k.clone(), v.last_access))
        .collect();
    entries.sort_by_key(|(_, t)| *t);

    let to_remove = entries.len().saturating_sub(keep_at_most);
    for i in 0..to_remove {
        if let Some((k, _)) = entries.get(i) {
            map.remove(k);
        }
    }
}
