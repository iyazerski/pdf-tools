use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// An uploaded PDF kept on disk; its directory is deleted when the last handle is dropped.
pub(crate) struct Upload {
    pub(crate) dir: TempDir,
    pub(crate) pdf_path: PathBuf,
    pub(crate) pages: usize,
}

struct UploadEntry {
    upload: Arc<Upload>,
    last_access: Instant,
}

pub(crate) struct UploadStore {
    ttl: Duration,
    max_entries: usize,
    inner: Mutex<HashMap<String, UploadEntry>>,
}

impl UploadStore {
    pub(crate) fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Delete expired uploads periodically, so files don't outlive the TTL while the app is idle.
    pub(crate) fn spawn_sweeper(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                interval.tick().await;
                self.sweep(Instant::now());
            }
        });
    }

    fn sweep(&self, now: Instant) {
        let expired: Vec<UploadEntry> = {
            let mut map = self.lock();
            let keys: Vec<String> = map
                .iter()
                .filter(|(_, e)| self.is_expired(e, now))
                .map(|(k, _)| k.clone())
                .collect();
            keys.iter().filter_map(|k| map.remove(k)).collect()
        };
        discard(expired);
    }

    pub(crate) fn put(&self, dir: TempDir, pdf_path: PathBuf, pages: usize) -> String {
        let upload_id = uuid::Uuid::new_v4().to_string();
        let entry = UploadEntry {
            upload: Arc::new(Upload {
                dir,
                pdf_path,
                pages,
            }),
            last_access: Instant::now(),
        };

        // Prefer bounded disk usage over keeping old uploads; the client can re-upload.
        let evicted = {
            let mut map = self.lock();
            let mut evicted = Vec::new();
            while map.len() >= self.max_entries {
                let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, e)| e.last_access)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                evicted.extend(map.remove(&oldest));
            }
            map.insert(upload_id.clone(), entry);
            evicted
        };
        discard(evicted);
        upload_id
    }

    pub(crate) fn get(&self, upload_id: &str) -> Option<Arc<Upload>> {
        let now = Instant::now();
        let mut map = self.lock();
        let entry = map.get_mut(upload_id)?;
        if self.is_expired(entry, now) {
            let expired = map.remove(upload_id);
            drop(map);
            discard(expired);
            return None;
        }
        entry.last_access = now;
        Some(entry.upload.clone())
    }

    pub(crate) fn remove(&self, upload_id: &str) {
        let removed = self.lock().remove(upload_id);
        discard(removed);
    }

    fn is_expired(&self, entry: &UploadEntry, now: Instant) -> bool {
        now.duration_since(entry.last_access) > self.ttl
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, UploadEntry>> {
        // The map stays consistent even if a holder panicked, so recover from poisoning.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Drop entries on the blocking pool, because deleting their directories is blocking I/O.
fn discard(entries: impl IntoIterator<Item = UploadEntry> + Send + 'static) {
    let entries: Vec<UploadEntry> = entries.into_iter().collect();
    if entries.is_empty() {
        return;
    }
    tokio::task::spawn_blocking(move || drop(entries));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_one(store: &UploadStore) -> (String, PathBuf) {
        let dir = TempDir::new().expect("tempdir must be created");
        let path = dir.path().to_path_buf();
        let id = store.put(dir, path.join("in.pdf"), 1);
        (id, path)
    }

    #[tokio::test]
    async fn sweep_deletes_expired_uploads_from_disk() {
        let store = UploadStore::new(Duration::from_secs(60), 10);
        let (id, path) = put_one(&store);

        store.sweep(Instant::now() + Duration::from_secs(61));
        tokio::task::yield_now().await;
        for _ in 0..100 {
            if !path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(store.get(&id).is_none());
        assert!(!path.exists(), "expired upload dir must be deleted");
    }

    #[tokio::test]
    async fn put_evicts_oldest_when_full() {
        let store = UploadStore::new(Duration::from_secs(60), 2);
        let (first, _) = put_one(&store);
        std::thread::sleep(Duration::from_millis(2));
        let (second, _) = put_one(&store);
        std::thread::sleep(Duration::from_millis(2));
        let (third, _) = put_one(&store);

        assert!(store.get(&first).is_none());
        assert!(store.get(&second).is_some());
        assert!(store.get(&third).is_some());
    }
}
