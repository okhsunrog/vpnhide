//! Read-only tools the model may call.
//!
//! They run in this process, against one checkout of the version the client
//! reported. There is no shell, no `git`, and no path that can leave that
//! checkout. `search_issues` does not call GitHub yet.

mod issues;
mod read_file;
mod sandbox;
mod search;

pub use issues::{IssueHit, IssueSearchOutput, IssueSearchStatus, search_issues};
pub use read_file::{FileWindow, ReadFileRequest, read_file};
pub use search::{SearchHit, SearchOutput, search_code, search_docs};

use serde::Serialize;

/// Why a tool refused to run. Messages stay generic so a path outside the
/// checkout is never echoed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolError {
    EmptyQuery,
    QueryTooLong,
    InvalidRange,
    Escapes,
    NotFound,
    NotAFile,
    Binary,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::EmptyQuery => "query is empty",
            Self::QueryTooLong => "query exceeds the tool limit",
            Self::InvalidRange => "offset or limit is not usable",
            Self::Escapes => "path leaves the checkout",
            Self::NotFound => "path was not found in the checkout",
            Self::NotAFile => "path is not a file",
            Self::Binary => "file is not text",
        };
        f.write_str(reason)
    }
}

impl std::error::Error for ToolError {}

pub(crate) fn prepare_query(query: &str, max_chars: usize) -> Result<String, ToolError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(ToolError::EmptyQuery);
    }
    if query.chars().count() > max_chars {
        return Err(ToolError::QueryTooLong);
    }
    Ok(query.to_lowercase())
}

/// Drop trailing matches until the JSON encoding fits. An empty value that is
/// still larger than the cap is returned as-is; there is nothing left to drop.
pub(crate) fn fit_json<T, F>(mut value: T, max_bytes: usize, mut drop_one: F) -> T
where
    T: Serialize,
    F: FnMut(&mut T) -> bool,
{
    loop {
        let encoded = serde_json::to_vec(&value).unwrap_or_default();
        if encoded.len() <= max_bytes || !drop_one(&mut value) {
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::ToolLimits;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempTree {
        path: PathBuf,
    }

    impl TempTree {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "vpnhide-ai-tools-{}-{}-{}",
                std::process::id(),
                nanos,
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("docs/help/en")).unwrap();
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(
                path.join("docs/help/en/gate.md"),
                "# Gate\n\nOnly a routed run measures hiding.\n",
            )
            .unwrap();
            fs::write(
                path.join("src/check.rs"),
                "fn classify() {\n    // routed gate\n}\n",
            )
            .unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn search_docs_and_search_code_stay_in_their_trees() {
        let tree = TempTree::new();
        let limits = ToolLimits::DEFAULT;
        let docs = search_docs(tree.path(), "routed", None, &limits).unwrap();
        assert_eq!(docs.matches.len(), 1);
        assert_eq!(docs.matches[0].path, "docs/help/en/gate.md");
        assert!(!docs.truncated);

        let code = search_code(tree.path(), "routed", None, &limits).unwrap();
        assert_eq!(code.matches.len(), 1);
        assert_eq!(code.matches[0].path, "src/check.rs");
    }

    #[test]
    fn search_response_is_capped() {
        let tree = TempTree::new();
        let mut body = String::new();
        for index in 0..8 {
            body.push_str(&format!(
                "needle {index} keeps going past the snippet cap and then some more text\n"
            ));
        }
        fs::write(tree.path().join("docs/help/en/noise.md"), &body).unwrap();
        let limits = ToolLimits {
            max_matches: 2,
            max_snippet_bytes: 24,
            max_response_bytes: 220,
            ..ToolLimits::DEFAULT
        };
        let output = search_docs(tree.path(), "needle", None, &limits).unwrap();
        assert!(output.truncated);
        assert!(output.matches.len() <= 2);
        assert!(output.matches.iter().all(|hit| hit.snippet.len() <= 24));
        let encoded = serde_json::to_vec(&output).unwrap();
        assert!(encoded.len() <= limits.max_response_bytes || output.matches.is_empty());
        assert!(output.matches.len() < 8);
    }

    #[test]
    fn read_file_rejects_paths_that_leave_the_checkout() {
        let tree = TempTree::new();
        // The secret sits next to the checkout, not inside it. Joining `..`
        // from the checkout would otherwise reach it.
        let parent = tree.path().parent().unwrap();
        let secret = parent.join(format!(
            "vpnhide-ai-secret-{}",
            tree.path().file_name().unwrap().to_string_lossy()
        ));
        fs::write(&secret, "superkey").unwrap();
        std::os::unix::fs::symlink(&secret, tree.path().join("leak")).unwrap();

        let limits = ToolLimits::DEFAULT;
        let samples = [
            "../".to_string() + &secret.file_name().unwrap().to_string_lossy(),
            secret.to_string_lossy().into_owned(),
            "docs/../../".to_string() + &secret.file_name().unwrap().to_string_lossy(),
            "leak".to_string(),
        ];
        for path in samples {
            let error = read_file(
                tree.path(),
                &ReadFileRequest {
                    path,
                    offset: 1,
                    limit: 20,
                },
                &limits,
            )
            .unwrap_err();
            assert_eq!(error, ToolError::Escapes);
        }
        let window = read_file(
            tree.path(),
            &ReadFileRequest {
                path: "docs/help/en/gate.md".to_string(),
                offset: 1,
                limit: 2,
            },
            &limits,
        )
        .unwrap();
        assert_eq!(window.lines.len(), 2);
        assert!(window.truncated);
        assert!(!window.eof);
        assert!(!window.lines.iter().any(|line| line.contains("superkey")));
        let _ = fs::remove_file(&secret);
    }

    #[test]
    fn issue_search_is_a_stub_and_still_bounded() {
        let error = search_issues("   ", &ToolLimits::DEFAULT).unwrap_err();
        assert_eq!(error, ToolError::EmptyQuery);
        let error = search_issues(&"a".repeat(201), &ToolLimits::DEFAULT).unwrap_err();
        assert_eq!(error, ToolError::QueryTooLong);

        let output = search_issues("ioctl_flags leak", &ToolLimits::DEFAULT).unwrap();
        assert_eq!(output.status, IssueSearchStatus::NotConfigured);
        assert!(output.matches.is_empty());
        let encoded = serde_json::to_vec(&output).unwrap();
        assert!(encoded.len() <= ToolLimits::DEFAULT.max_response_bytes);
    }
}
