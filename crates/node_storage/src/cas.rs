//! Content-Addressed Store (CAS).
//!
//! Stores blobs under `data/objects/sha256/xx/yy/<full_hash>`.
//! Deduplication is inherent: identical content maps to the same hash.
//! Integrity is verified on read.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use node_proto::common::HashRef;

#[derive(Debug, Error)]
pub enum CasError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("integrity check failed for {hash}: expected {hash}, got {actual}")]
    IntegrityFailure { hash: String, actual: String },
    #[error("object not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, CasError>;

pub struct CasStore {
    root: PathBuf,
}

impl CasStore {
    /// Create a new CAS store rooted at `root/objects/sha256`.
    pub fn open(root: &Path) -> Result<Self> {
        let objects_root = root.join("objects").join("sha256");
        fs::create_dir_all(&objects_root)?;
        Ok(Self { root: objects_root })
    }

    fn object_path(&self, hash: &str) -> PathBuf {
        let (prefix, rest) = hash.split_at(2.min(hash.len()));
        let (mid, _tail) = if rest.len() >= 2 {
            rest.split_at(2)
        } else {
            (rest, "")
        };
        self.root.join(prefix).join(mid).join(hash)
    }

    /// Store bytes and return the SHA-256 HashRef. Deduplicates automatically.
    pub fn put_bytes(&self, _content_type: &str, data: &[u8]) -> Result<HashRef> {
        let hash = hex::encode(Sha256::digest(data));
        let path = self.object_path(&hash);

        if path.exists() {
            tracing::debug!(hash = %hash, "CAS dedup hit");
            return Ok(HashRef { sha256: hash });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("tmp");
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);

        fs::rename(&tmp_path, &path)?;
        tracing::debug!(hash = %hash, size = data.len(), "CAS object stored");

        Ok(HashRef { sha256: hash })
    }

    /// Retrieve bytes by hash. Verifies integrity on read.
    pub fn get_bytes(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Err(CasError::NotFound(hash.to_string()));
        }

        let data = fs::read(&path)?;
        let actual = hex::encode(Sha256::digest(&data));
        if actual != hash {
            return Err(CasError::IntegrityFailure {
                hash: hash.to_string(),
                actual,
            });
        }

        Ok(data)
    }

    /// Check whether a hash exists in the store.
    pub fn has(&self, hash: &str) -> bool {
        self.object_path(hash).exists()
    }

    /// Return the size in bytes of a stored object, or None if absent.
    pub fn size(&self, hash: &str) -> Result<Option<u64>> {
        let path = self.object_path(hash);
        if !path.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&path)?;
        Ok(Some(meta.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, CasStore) {
        let tmp = TempDir::new().unwrap();
        let store = CasStore::open(tmp.path()).unwrap();
        (tmp, store)
    }

    #[test]
    fn put_and_get() {
        let (_tmp, store) = setup();
        let data = b"hello, world!";
        let href = store.put_bytes("text/plain", data).unwrap();
        assert!(!href.sha256.is_empty());

        let retrieved = store.get_bytes(&href.sha256).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn dedup() {
        let (_tmp, store) = setup();
        let data = b"duplicate me";
        let h1 = store.put_bytes("text/plain", data).unwrap();
        let h2 = store.put_bytes("text/plain", data).unwrap();
        assert_eq!(h1.sha256, h2.sha256);
    }

    #[test]
    fn has_check() {
        let (_tmp, store) = setup();
        let data = b"existence check";
        let href = store.put_bytes("text/plain", data).unwrap();
        assert!(store.has(&href.sha256));
        assert!(!store.has("nonexistent_hash"));
    }

    #[test]
    fn not_found() {
        let (_tmp, store) = setup();
        let err = store.get_bytes("deadbeef").unwrap_err();
        assert!(matches!(err, CasError::NotFound(_)));
    }

    #[test]
    fn corruption_detection() {
        let (_tmp, store) = setup();
        let data = b"original content";
        let href = store.put_bytes("text/plain", data).unwrap();

        let path = store.object_path(&href.sha256);
        fs::write(&path, b"corrupted!").unwrap();

        let err = store.get_bytes(&href.sha256).unwrap_err();
        assert!(matches!(err, CasError::IntegrityFailure { .. }));
    }

    #[test]
    fn empty_content() {
        let (_tmp, store) = setup();
        let href = store.put_bytes("application/octet-stream", b"").unwrap();
        assert_eq!(
            href.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let retrieved = store.get_bytes(&href.sha256).unwrap();
        assert!(retrieved.is_empty());
    }

    #[test]
    fn large_content() {
        let (_tmp, store) = setup();
        let data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        let href = store.put_bytes("application/octet-stream", &data).unwrap();
        let retrieved = store.get_bytes(&href.sha256).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn size_check() {
        let (_tmp, store) = setup();
        let data = b"size test data";
        let href = store.put_bytes("text/plain", data).unwrap();
        assert_eq!(store.size(&href.sha256).unwrap(), Some(data.len() as u64));
        assert_eq!(store.size("nonexistent").unwrap(), None);
    }

    #[test]
    fn multiple_objects() {
        let (_tmp, store) = setup();
        let mut hashes = Vec::new();
        for i in 0..100 {
            let data = format!("object number {i}");
            let href = store.put_bytes("text/plain", data.as_bytes()).unwrap();
            hashes.push((href.sha256, data));
        }
        for (hash, original) in &hashes {
            let retrieved = store.get_bytes(hash).unwrap();
            assert_eq!(retrieved, original.as_bytes());
        }
    }
}
