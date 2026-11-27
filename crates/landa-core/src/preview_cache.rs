use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Default)]
struct QuotaState {
    reserved_bytes: u64,
    active_paths: HashSet<PathBuf>,
}

#[derive(Default)]
pub struct PreviewCacheQuota {
    inner: Mutex<QuotaState>,
}

pub struct PreviewReservation<'a> {
    quota: &'a PreviewCacheQuota,
    reserved_bytes: u64,
    active_paths: Vec<PathBuf>,
}

impl Drop for PreviewReservation<'_> {
    fn drop(&mut self) {
        let mut state = self.quota.inner.lock().expect("preview quota lock poisoned");
        state.reserved_bytes = state.reserved_bytes.saturating_sub(self.reserved_bytes);
        for path in &self.active_paths {
            state.active_paths.remove(path);
        }
    }
}

impl PreviewCacheQuota {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_paths_snapshot(&self) -> HashSet<PathBuf> {
        self.inner
            .lock()
            .expect("preview quota lock poisoned")
            .active_paths
            .clone()
    }

    pub fn reserve(
        &self,
        dir: &Path,
        needed_bytes: u64,
        max_bytes: u64,
        active_paths: &[PathBuf],
    ) -> Result<PreviewReservation<'_>, std::io::Error> {
        if needed_bytes > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "file exceeds preview cache quota",
            ));
        }

        std::fs::create_dir_all(dir)?;
        let mut state = self.inner.lock().expect("preview quota lock poisoned");

        let mut completed_bytes: u64 = 0;
        let mut evictable: Vec<(PathBuf, u64, SystemTime)> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let is_part = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(".preview-part-"));
                if is_part {
                    // Active .preview-part- files are already accounted for in state.reserved_bytes.
                    continue;
                }
                let len = meta.len();
                completed_bytes = completed_bytes.saturating_add(len);
                if !state.active_paths.contains(&path) && !active_paths.contains(&path) {
                    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    evictable.push((path, len, modified));
                }
            }
        }

        let mut total_committed = completed_bytes.saturating_add(state.reserved_bytes);
        if total_committed.saturating_add(needed_bytes) > max_bytes {
            evictable.sort_by_key(|(_, _, modified)| *modified);
            for (path, len, _) in evictable {
                if total_committed.saturating_add(needed_bytes) <= max_bytes {
                    break;
                }
                if std::fs::remove_file(&path).is_ok() {
                    total_committed = total_committed.saturating_sub(len);
                }
            }
        }

        if total_committed.saturating_add(needed_bytes) > max_bytes {
            return Err(std::io::Error::other(
                "preview cache quota exceeded by active downloads",
            ));
        }

        state.reserved_bytes = state.reserved_bytes.saturating_add(needed_bytes);
        for path in active_paths {
            state.active_paths.insert(path.clone());
        }

        Ok(PreviewReservation {
            quota: self,
            reserved_bytes: needed_bytes,
            active_paths: active_paths.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parallel_reservations_enforce_hard_disk_quota_before_writing() {
        let dir = std::env::temp_dir().join(format!("landa-quota-par-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let quota = PreviewCacheQuota::new();
        let max_bytes = 150;

        // Existing completed file of 100 bytes.
        let existing = dir.join("existing.bin");
        std::fs::write(&existing, vec![1u8; 100]).unwrap();

        // First download needs 100 bytes: evicts existing (100B) before writing, reserves 100B.
        let part1 = dir.join(".preview-part-1");
        let res1 = quota
            .reserve(&dir, 100, max_bytes, std::slice::from_ref(&part1))
            .expect("first 100B reservation should succeed after evicting existing 100B");
        assert!(!existing.exists(), "existing file must be evicted before writing");
        std::fs::write(&part1, vec![2u8; 100]).unwrap();

        // Second parallel download needs 100 bytes: 100 + 100 = 200 > 150 -> must fail BEFORE writing!
        let part2 = dir.join(".preview-part-2");
        let err = quota
            .reserve(&dir, 100, max_bytes, std::slice::from_ref(&part2))
            .err()
            .expect("second parallel 100B reservation must fail when 100B is already reserved");
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
        assert!(!part2.exists(), "rejected download must not write to disk");

        let disk_total: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum();
        assert!(
            disk_total <= max_bytes,
            "disk usage during parallel downloads ({disk_total}) must never exceed {max_bytes}"
        );

        drop(res1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
