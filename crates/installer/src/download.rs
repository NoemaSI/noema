//! Downloading, caching and verifying pinned binary assets.
//!
//! All network access goes through the [`Fetcher`] trait so tests can
//! run the full install flow with a stub — no network, no real
//! binaries. Downloads land in `<root>/downloads/<sha256>` and are
//! reused when already present and intact; a full setup prunes the
//! cache at the end.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::manifest::Asset;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    ChecksumMismatch { url: String, expected: String, actual: String },
}

/// Whether fetched bytes must hash to the manifest digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verification {
    /// Always used by the `noema-setup` binary: every byte is checked
    /// against the pinned sha256.
    #[default]
    Enforce,
    /// For in-crate tests that serve synthetic binaries through a
    /// stub [`Fetcher`]. Never reachable from the CLI.
    TrustFetcher,
}

/// Abstraction over HTTP so the install flow is testable offline.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

/// Production fetcher: plain HTTPS GET with redirect following.
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let mut response = ureq::get(url)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build()
            .call()
            .with_context(|| format!("GET {url}"))?;
        let bytes = response
            .body_mut()
            .read_to_vec()
            .with_context(|| format!("reading body of {url}"))?;
        Ok(bytes)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Verify `bytes` against the manifest digest.
pub fn verify_sha256(bytes: &[u8], url: &str, expected: &str) -> Result<(), DownloadError> {
    let actual = sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(DownloadError::ChecksumMismatch {
            url: url.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

/// Fetch an asset into the cache, reusing a previously cached copy
/// when it hashes to the pinned digest. Returns the cache path.
pub fn fetch_cached(
    fetcher: &dyn Fetcher,
    asset: &Asset,
    downloads: &Path,
    verification: Verification,
) -> anyhow::Result<PathBuf> {
    let check = |bytes: &[u8]| match verification {
        Verification::Enforce => verify_sha256(bytes, asset.url, asset.sha256),
        Verification::TrustFetcher => Ok(()),
    };
    std::fs::create_dir_all(downloads)
        .with_context(|| format!("creating download cache {}", downloads.display()))?;
    let cached = downloads.join(asset.sha256);
    if let Ok(existing) = std::fs::read(&cached) {
        if check(&existing).is_ok() {
            return Ok(cached);
        }
    }
    let bytes = fetcher.fetch(asset.url)?;
    check(&bytes)?;
    write_atomic(&cached, &bytes).with_context(|| format!("caching {}", cached.display()))?;
    Ok(cached)
}

/// Copy a cached asset to its install destination and make it
/// executable.
pub fn install_binary(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let staging = destination.with_extension("part");
    let bytes = std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
    let mut file = std::fs::File::create(&staging)
        .with_context(|| format!("creating {}", staging.display()))?;
    file.write_all(&bytes)?;
    file.flush()?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod {}", staging.display()))?;
    }
    #[cfg(windows)]
    {
        // ReplaceFile semantics on a running .exe are unreliable; drop
        // the old copy first and ignore its absence.
        let _ = std::fs::remove_file(destination);
    }
    std::fs::rename(&staging, destination).with_context(|| {
        format!(
            "moving {} to {}",
            staging.display(),
            destination.display()
        )
    })
}

/// Remove everything from the download cache (kept until a full setup
/// succeeds so a repair run never re-downloads).
pub fn prune_cache(downloads: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(downloads) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow::Error::new(error)
            .context(format!("pruning {}", downloads.display()))),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let staging = path.with_extension("part");
    std::fs::write(&staging, bytes)?;
    std::fs::rename(&staging, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    pub struct StubFetcher {
        pub body: Vec<u8>,
        pub calls: Cell<usize>,
    }

    impl StubFetcher {
        pub fn new(body: &[u8]) -> Self {
            Self { body: body.to_vec(), calls: Cell::new(0) }
        }
    }

    impl Fetcher for StubFetcher {
        fn fetch(&self, _url: &str) -> anyhow::Result<Vec<u8>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.body.clone())
        }
    }

    fn asset(sha256: &'static str) -> Asset {
        Asset { name: "rumoca-linux-x86_64", url: "https://example.invalid/binary", sha256 }
    }

    #[test]
    fn sha256_matches_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_reports_mismatch() {
        let error = verify_sha256(b"abc", "https://example.invalid/x", &"0".repeat(64))
            .unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
    }

    #[test]
    fn caches_by_digest_and_reuses_without_refetching() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("downloads");
        let sha = sha256_hex(b"payload");
        let fetcher = StubFetcher::new(b"payload");
        let a = asset(Box::leak(sha.clone().into_boxed_str()));

        let first = fetch_cached(&fetcher, &a, &downloads, Verification::Enforce).unwrap();
        let second = fetch_cached(&fetcher, &a, &downloads, Verification::Enforce).unwrap();
        assert_eq!(first, second);
        assert_eq!(fetcher.calls.get(), 1, "second fetch must hit the cache");
        assert_eq!(std::fs::read(&first).unwrap(), b"payload");
    }

    #[test]
    fn corrupt_cache_entry_is_refetched() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("downloads");
        let sha = sha256_hex(b"payload");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join(&sha), b"corrupt").unwrap();
        let fetcher = StubFetcher::new(b"payload");
        let a = asset(Box::leak(sha.into_boxed_str()));

        let path = fetch_cached(&fetcher, &a, &downloads, Verification::Enforce).unwrap();
        assert_eq!(fetcher.calls.get(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    }

    #[test]
    fn installs_executable_binary() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("asset");
        std::fs::write(&source, b"#!/bin/sh\necho hi\n").unwrap();
        let destination = dir.path().join("nested").join("bin").join("tool");

        install_binary(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"#!/bin/sh\necho hi\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(std::fs::metadata(&destination).unwrap().permissions().mode() & 0o111 > 0);
        }
    }
}