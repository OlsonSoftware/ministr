//! Deterministic, cross-platform corpus identity.
//!
//! This module is the **single source of truth** for turning a set of raw
//! corpus path strings into a stable `corpus_id`. The daemon
//! (`ministr-daemon`'s registry) and the CLI (`ministr-cli`'s infra) both
//! resolve a corpus's on-disk data to `<data_dir>/corpora/<corpus_id>`, so
//! if the two derived different names for the same project the CLI-indexed
//! data and the daemon-served data would silently diverge into two
//! directories — data loss with no error. Keeping the derivation here, used
//! by both, makes that class of bug structurally impossible.
//!
//! Canonicalisation rules for paths classified as
//! [`CorpusSource::Local`](crate::config::CorpusSource):
//!
//! - Windows extended-length / UNC *verbatim* prefixes (`\\?\`, `\\?\UNC\`)
//!   are stripped — `\\?\C:\foo` and `C:\foo` are the same location.
//! - `\` becomes `/` so Windows and Unix forms hash identically.
//! - Trailing `/` is stripped (but the lone `/` root is preserved).
//! - On Windows the result is lowercased — NTFS is case-insensitive, so
//!   `D:/Code/foo` and `d:/code/foo` must be one corpus, not two.
//!
//! Non-local paths (HTTP, git, `github://`) pass through unchanged so
//! remote-URL identity isn't accidentally rewritten.
//!
//! Empty or whitespace-only inputs are rejected with [`CorpusIdError`]
//! instead of silently collapsing to a junk identity.

use std::borrow::Cow;

use sha2::{Digest, Sha256};

use crate::config::{CorpusSource, classify_corpus_path};

/// Error deriving a corpus identity from a path set.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CorpusIdError {
    /// The path set was empty, or a member was empty / whitespace-only.
    #[error("corpus path set is empty or contains an empty path")]
    EmptyPath,
}

/// Strip a Windows verbatim (`\\?\`) or verbatim-UNC (`\\?\UNC\`) prefix.
///
/// `\\?\C:\foo` → `C:\foo`; `\\?\UNC\server\share` → `\\server\share`.
/// Anything else is returned unchanged. This is a no-op for ordinary
/// paths, so it cannot change the id of an already-registered corpus.
fn strip_windows_verbatim(raw: &str) -> Cow<'_, str> {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        // Re-form as a plain UNC path so it canonicalises identically to
        // the non-verbatim spelling of the same share.
        Cow::Owned(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        Cow::Borrowed(rest)
    } else {
        Cow::Borrowed(raw)
    }
}

/// Lexically canonicalise a single corpus path string.
///
/// See the [module docs](self) for the full rule set. Non-local sources
/// pass through unchanged.
#[must_use]
pub fn canonical_corpus_path(raw: &str) -> String {
    if !matches!(classify_corpus_path(raw), CorpusSource::Local(_)) {
        return raw.to_owned();
    }

    let stripped = strip_windows_verbatim(raw);
    let normalised_seps = stripped.replace('\\', "/");
    let mut s = lexical_clean(&normalised_seps);

    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }

    #[cfg(windows)]
    {
        s = s.to_lowercase();
    }

    s
}

/// Lexically clean a slash-separated path:
/// - drop `.` segments
/// - resolve `..` against the previous segment when safe (never pops past
///   an absolute root or a UNC root)
/// - collapse repeated `/`
///
/// Operates purely on the input string — no filesystem syscalls. Returns
/// the input unchanged when there are no `.`/`..`/`//` segments, so two
/// path spellings that differ only in those components canonicalise to
/// the same identity and don't aliase into duplicate corpora.
fn lexical_clean(s: &str) -> String {
    // UNC (`//server/share`) vs single-root absolute (`/foo`) vs relative.
    let unc = s.starts_with("//");
    let absolute = !unc && s.starts_with('/');

    let mut segs: Vec<&str> = Vec::new();
    for part in s.split('/') {
        match part {
            "" | "." => {}
            ".." => match segs.last() {
                Some(&last) if last != ".." => {
                    segs.pop();
                }
                _ => {
                    // Can't pop past root — drop `..` on absolute / UNC
                    // paths; keep on relative ones so they're still
                    // navigable.
                    if !absolute && !unc {
                        segs.push("..");
                    }
                }
            },
            other => segs.push(other),
        }
    }
    let joined = segs.join("/");
    if unc {
        format!("//{joined}")
    } else if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Canonicalise, sort, and dedup a corpus path set.
///
/// The result is the input both to [`corpus_id_from_paths`] and to the
/// daemon's stored `CorpusInfo.paths`, so two equivalent path sets produce
/// identical ids, identical on-disk dirs, and identical manifest entries.
///
/// # Errors
///
/// Returns [`CorpusIdError::EmptyPath`] if `paths` is empty or any member
/// is empty / whitespace-only — these previously collapsed to a junk
/// identity (`""` → `/`), silently aliasing unrelated corpora.
pub fn canonical_corpus_paths(paths: &[String]) -> Result<Vec<String>, CorpusIdError> {
    if paths.is_empty() {
        return Err(CorpusIdError::EmptyPath);
    }
    let mut out: Vec<String> = Vec::with_capacity(paths.len());
    for p in paths {
        if p.trim().is_empty() {
            return Err(CorpusIdError::EmptyPath);
        }
        out.push(canonical_corpus_path(p));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Derive a deterministic corpus ID from a path set.
///
/// Paths are canonicalised first via [`canonical_corpus_paths`] so that
/// equivalent inputs (case, separator, trailing-slash, verbatim-prefix,
/// ordering variants) hash to the same id. The wire format
/// (`multi-<first 8 hex of sha256 of the `\n`-joined canonical paths>`)
/// is intentionally unchanged from the daemon's historical scheme so
/// existing on-disk corpora keep resolving to their current directory.
///
/// # Errors
///
/// Propagates [`CorpusIdError`] from [`canonical_corpus_paths`].
pub fn corpus_id_from_paths(paths: &[String]) -> Result<String, CorpusIdError> {
    use std::fmt::Write as _;
    let canonical = canonical_corpus_paths(paths)?;
    let hash = Sha256::digest(canonical.join("\n").as_bytes());
    let hex = hash.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    });
    Ok(format!("multi-{}", &hex[..8]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_blank_paths() {
        assert_eq!(corpus_id_from_paths(&[]), Err(CorpusIdError::EmptyPath));
        assert_eq!(
            corpus_id_from_paths(&[String::new()]),
            Err(CorpusIdError::EmptyPath)
        );
        assert_eq!(
            corpus_id_from_paths(&["   ".into()]),
            Err(CorpusIdError::EmptyPath)
        );
    }

    #[test]
    fn order_and_dedup_invariant() {
        let a = corpus_id_from_paths(&["b".into(), "a".into(), "a".into()]).unwrap();
        let b = corpus_id_from_paths(&["a".into(), "b".into()]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn trailing_slash_invariant() {
        assert_eq!(canonical_corpus_path("/users/x/foo/"), "/users/x/foo");
        assert_eq!(canonical_corpus_path("/users/x/foo"), "/users/x/foo");
        assert_eq!(canonical_corpus_path("/"), "/");
        let t = corpus_id_from_paths(&["/users/x/foo/".into()]).unwrap();
        let n = corpus_id_from_paths(&["/users/x/foo".into()]).unwrap();
        assert_eq!(t, n);
    }

    #[test]
    fn separators_normalised() {
        // Backslash → forward slash on every platform so a project
        // committed on Windows resolves to the same id on macOS / Linux.
        let win = canonical_corpus_path("D:\\Code\\ministr");
        assert!(win == "d:/code/ministr" || win == "D:/Code/ministr");
    }

    #[test]
    fn remote_urls_pass_through() {
        let raw = "https://Example.com/Some/Path/";
        assert_eq!(canonical_corpus_path(raw), raw);
        let git = "git@github.com:User/Repo.git";
        assert_eq!(canonical_corpus_path(git), git);
    }

    #[test]
    fn windows_verbatim_prefix_stripped() {
        // `\\?\C:\foo` and `C:\foo` are the same location and must
        // produce the same id regardless of host OS.
        assert_eq!(
            corpus_id_from_paths(&[r"\\?\C:\Code\foo".into()]).unwrap(),
            corpus_id_from_paths(&[r"C:\Code\foo".into()]).unwrap()
        );
        // Verbatim-UNC collapses to the plain UNC spelling.
        assert_eq!(
            canonical_corpus_path(r"\\?\UNC\server\share"),
            canonical_corpus_path(r"\\server\share")
        );
    }

    #[test]
    fn sibling_projects_distinct() {
        let a = corpus_id_from_paths(&["/code/foo".into()]).unwrap();
        let b = corpus_id_from_paths(&["/code/bar".into()]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn trailing_dot_invariant() {
        // `<dir>` and `<dir>/.` are the same location; both must hash to
        // the same id so tray-app users don't see two entries for the
        // same project (the original bug that motivated this rule).
        assert_eq!(canonical_corpus_path("/users/x/foo/."), "/users/x/foo");
        let a = corpus_id_from_paths(&["/users/x/foo".into()]).unwrap();
        let b = corpus_id_from_paths(&["/users/x/foo/.".into()]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn interior_dot_invariant() {
        assert_eq!(canonical_corpus_path("/users/x/./foo"), "/users/x/foo");
        let a = corpus_id_from_paths(&["/users/x/foo".into()]).unwrap();
        let b = corpus_id_from_paths(&["/users/x/./foo".into()]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn parent_dir_resolved() {
        assert_eq!(canonical_corpus_path("/users/x/foo/../bar"), "/users/x/bar");
        assert_eq!(canonical_corpus_path("/foo/.."), "/");
    }

    #[test]
    fn parent_dir_does_not_escape_root() {
        // `..` past `/` is dropped — the path stays at the filesystem
        // root rather than turning into an empty / relative spelling.
        assert_eq!(canonical_corpus_path("/../foo"), "/foo");
        assert_eq!(canonical_corpus_path("/../../foo"), "/foo");
    }

    #[test]
    fn repeated_slashes_collapsed() {
        assert_eq!(canonical_corpus_path("/users//x/foo"), "/users/x/foo");
        let a = corpus_id_from_paths(&["/users/x/foo".into()]).unwrap();
        let b = corpus_id_from_paths(&["/users//x/foo".into()]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unc_root_preserved() {
        assert_eq!(
            canonical_corpus_path("//server/share/sub"),
            "//server/share/sub"
        );
        // Even with a trailing dot.
        assert_eq!(canonical_corpus_path("//server/share/."), "//server/share");
    }

    #[test]
    fn relative_parent_dirs_kept() {
        // Relative paths can't be resolved without filesystem context, so
        // leading `..` segments survive canonicalisation.
        assert_eq!(canonical_corpus_path("../foo"), "../foo");
        assert_eq!(canonical_corpus_path("./foo"), "foo");
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_insensitive() {
        assert_eq!(
            canonical_corpus_path("D:\\Code\\Ministr"),
            "d:/code/ministr"
        );
        let a = corpus_id_from_paths(&["D:\\Code\\Ministr".into()]).unwrap();
        let b = corpus_id_from_paths(&["d:/code/ministr/".into()]).unwrap();
        assert_eq!(a, b);
    }
}
