//! Undo blob store.
//!
//! Stores file snapshots before write/edit operations.
//! GC runs every 10 saves, purging blobs older than 24 hours.
//!
//! Store location: `~/.config/openrust/undo/`
//! Blob naming: `{sha256_hex}` — content is the original file content.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug)]
pub struct UndoStore {
    dir: PathBuf,
    save_count: Mutex<u32>,
}

impl UndoStore {
    pub fn new() -> Self {
        let dir = crate::core::config::Config::global_config_dir().join("undo");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir, save_count: Mutex::new(0) }
    }

    /// Save a file snapshot and return the undo hash.
    /// Returns None if the file doesn't exist or can't be read.
    pub fn save_snapshot(&self, file_path: &Path, file_content: &str) -> Option<String> {
        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(file_path.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            hasher.update(file_content.as_bytes());
            hex::encode(hasher.finalize())
        };

        let blob_path = self.dir.join(&hash);
        if !blob_path.exists() {
            let _ = std::fs::write(&blob_path, file_content);
        }

        // GC check every 10 saves
        let mut count = self.save_count.lock().unwrap();
        *count += 1;
        if *count % 10 == 0 {
            drop(count);
            self.gc();
        }

        Some(hash)
    }

    /// Read a blob by hash.
    pub fn read_blob(&self, hash: &str) -> Option<String> {
        // Sanitize: prevent path traversal
        if hash.contains('/') || hash.contains("..") {
            return None;
        }
        let path = self.dir.join(hash);
        std::fs::read_to_string(&path).ok()
    }

    /// Delete a blob by hash.
    pub fn delete_blob(&self, hash: &str) {
        if hash.contains('/') || hash.contains("..") {
            return;
        }
        let path = self.dir.join(hash);
        let _ = std::fs::remove_file(&path);
    }

    /// Garbage collect: remove blobs older than 24 hours.
    fn gc(&self) {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(24 * 3600))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if modified < cutoff {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
}

impl Default for UndoStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_read_blob() {
        let store = UndoStore::new();
        let hash = store.save_snapshot(Path::new("test.txt"), "hello world").unwrap();
        let content = store.read_blob(&hash).unwrap();
        assert_eq!(content, "hello world");
        store.delete_blob(&hash);
    }

    #[test]
    fn test_different_content_different_hash() {
        let store = UndoStore::new();
        let h1 = store.save_snapshot(Path::new("a.txt"), "a").unwrap();
        let h2 = store.save_snapshot(Path::new("a.txt"), "b").unwrap();
        assert_ne!(h1, h2);
        store.delete_blob(&h1);
        store.delete_blob(&h2);
    }

    #[test]
    fn test_path_traversal_prevented() {
        let store = UndoStore::new();
        assert!(store.read_blob("../etc/passwd").is_none());
        assert!(store.read_blob("foo/bar").is_none());
    }
}
