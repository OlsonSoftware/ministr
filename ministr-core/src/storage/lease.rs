//! Index-directory ownership lease — one writer per index dir.
//!
//! Nothing structural used to stop two engines (the shared daemon and an
//! in-process CLI engine, or two daemons) from opening the same corpus
//! directory and double-writing `content.db` + the vector index — the bug
//! class behind orphaned index dirs and cross-writer corruption. The lease
//! makes single-writer ownership a property of the directory itself:
//!
//! - Every writer acquires [`IndexLease::acquire`] on the dir before opening
//!   storage. The daemon holds one per corpus handle; an in-process engine
//!   holds one for the life of the command.
//! - The lock is an OS advisory lock (`std::fs::File::try_lock` — `flock` on
//!   Unix, `LockFileEx` on Windows) on a `LOCK` file inside the dir. The
//!   kernel releases it when the holding process exits **for any reason**,
//!   so staleness handling is structural: a lease can never outlive its
//!   holder, and a `LOCK` file left behind by a crash is inert.
//! - The file's *contents* (pid + holder description) are diagnostics only —
//!   they name the probable holder in the busy error so callers can say, in
//!   plain words, who owns the index and what to do instead.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Name of the lock file inside a corpus/index directory.
pub const LEASE_FILE_NAME: &str = "LOCK";

/// Why an ownership lease could not be acquired.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    /// Another live process holds the lease on this index directory.
    #[error("index at {dir} is owned by another process ({holder})")]
    Held {
        /// The contested index directory.
        dir: PathBuf,
        /// Best-effort description of the holder, read from the lock file
        /// (e.g. `"ministr daemon, pid 512"`); `"unknown process"` when the
        /// contents are unreadable.
        holder: String,
    },

    /// Creating or locking the lease file failed for a non-contention reason.
    #[error("lease file at {dir}: {source}")]
    Io {
        /// The index directory the lease file lives in.
        dir: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive ownership lease on one index directory.
///
/// Held for as long as the value lives; dropping it (or the process dying)
/// releases the OS lock. The `LOCK` file itself is left in place — its
/// existence carries no meaning, only the live lock does.
#[derive(Debug)]
pub struct IndexLease {
    /// Keeps the locked file descriptor open; the OS lock lives on it.
    _file: File,
    dir: PathBuf,
}

impl IndexLease {
    /// Acquire the exclusive writer lease for `corpus_dir`, without blocking.
    ///
    /// `holder` describes *this* acquirer (e.g. `"ministr daemon"`); it is
    /// written into the lock file so a later contender's error can name us.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::Held`] — a live process owns the dir; the error names
    ///   it from the lock-file diagnostics.
    /// - [`LeaseError::Io`] — the dir or lock file could not be created,
    ///   opened, or written.
    pub fn acquire(corpus_dir: &Path, holder: &str) -> Result<Self, LeaseError> {
        let io = |source| LeaseError::Io {
            dir: corpus_dir.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(corpus_dir).map_err(io)?;
        let path = corpus_dir.join(LEASE_FILE_NAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io)?;

        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(LeaseError::Held {
                    dir: corpus_dir.to_path_buf(),
                    holder: read_holder(&mut file),
                });
            }
            Err(std::fs::TryLockError::Error(source)) => return Err(io(source)),
        }

        // Locked — record who we are, purely as diagnostics for the next
        // contender's error message. Failure to write is not failure to own.
        let note = format!("{holder}, pid {}", std::process::id());
        let _ = file.set_len(0);
        let _ = file.write_all(note.as_bytes());
        let _ = file.flush();

        Ok(Self {
            _file: file,
            dir: corpus_dir.to_path_buf(),
        })
    }

    /// The directory this lease owns.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Best-effort holder description from an already-open lock file.
fn read_holder(file: &mut File) -> String {
    let mut holder = String::new();
    if file.read_to_string(&mut holder).is_ok() {
        let holder = holder.trim();
        if !holder.is_empty() {
            return holder.to_owned();
        }
    }
    "unknown process".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_succeeds_on_fresh_dir_and_writes_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("corpus-a");

        let lease = IndexLease::acquire(&dir, "unit test").unwrap();
        assert_eq!(lease.dir(), dir);

        let contents = std::fs::read_to_string(dir.join(LEASE_FILE_NAME)).unwrap();
        assert!(contents.starts_with("unit test, pid "), "got: {contents}");
    }

    #[test]
    fn second_acquire_in_same_process_is_refused_and_names_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        let _held = IndexLease::acquire(&dir, "first owner").unwrap();

        // flock/LockFileEx conflict across separate descriptors, even within
        // one process — exactly the cross-engine contention we guard against.
        match IndexLease::acquire(&dir, "second owner") {
            Err(LeaseError::Held { holder, .. }) => {
                assert!(holder.starts_with("first owner, pid "), "got: {holder}");
            }
            other => panic!("expected Held, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_lease_frees_the_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        drop(IndexLease::acquire(&dir, "first").unwrap());
        let relocked = IndexLease::acquire(&dir, "second").unwrap();
        assert_eq!(relocked.dir(), dir);
    }

    #[test]
    fn stale_lock_file_without_live_holder_is_inert() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        // Simulate a crashed holder: LOCK exists with contents, no live lock.
        std::fs::write(dir.join(LEASE_FILE_NAME), "dead daemon, pid 1").unwrap();

        let lease = IndexLease::acquire(&dir, "survivor").unwrap();
        assert_eq!(lease.dir(), dir);
    }
}
