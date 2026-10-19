//! Literal, case-insensitive search over one checkout.
//!
//! Docs and code are separate walks so a question about a help article does
//! not come back as a hit in `docs/` mixed with a hit in the sources. The
//! query is a substring, not a shell pattern and not a regular expression.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;

use super::sandbox::{relative_display, resolve_inside};
use super::{ToolError, fit_json, prepare_query};
use crate::limits::{ToolLimits, truncate_bytes};

const SKIP_DIRS: &[&str] = &[".git", "target", ".gradle", "build", "node_modules"];
const CODE_EXTENSIONS: &[&str] = &[
    "rs",
    "kt",
    "kts",
    "c",
    "h",
    "py",
    "toml",
    "json",
    "md",
    "txt",
    "xml",
    "properties",
    "yml",
    "yaml",
    "sh",
    "gradle",
];
const DOC_EXTENSIONS: &[&str] = &["md", "txt", "json"];

/// One matching line. `path` is relative to the checkout and uses `/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    pub snippet: String,
}

/// Bounded search result. `truncated` means the cap hid further hits, files,
/// or bytes — not that the query was rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchOutput {
    pub matches: Vec<SearchHit>,
    pub truncated: bool,
    pub files_scanned: u32,
    pub files_skipped: u32,
}

/// Search `docs/` of this checkout, including `docs/help/`. `path_prefix` is
/// relative to `docs/`, for example `help/en`.
pub fn search_docs(
    checkout: &Path,
    query: &str,
    path_prefix: Option<&str>,
    limits: &ToolLimits,
) -> Result<SearchOutput, ToolError> {
    let needle = prepare_query(query, limits.max_query_chars)?;
    let docs = checkout.join("docs");
    if !docs.is_dir() {
        return Ok(empty_output());
    }
    let start = match path_prefix {
        Some(prefix) => resolve_inside(&docs, prefix)?,
        None => docs.canonicalize().map_err(|_| ToolError::NotFound)?,
    };
    scan(checkout, &start, &needle, DOC_EXTENSIONS, false, limits)
}

/// Search the checkout of this version, excluding `docs/` (use [`search_docs`])
/// and excluding VCS and build trees.
pub fn search_code(
    checkout: &Path,
    query: &str,
    path_prefix: Option<&str>,
    limits: &ToolLimits,
) -> Result<SearchOutput, ToolError> {
    let needle = prepare_query(query, limits.max_query_chars)?;
    let start = match path_prefix {
        Some(prefix) => resolve_inside(checkout, prefix)?,
        None => checkout.canonicalize().map_err(|_| ToolError::NotFound)?,
    };
    scan(checkout, &start, &needle, CODE_EXTENSIONS, true, limits)
}

fn empty_output() -> SearchOutput {
    SearchOutput {
        matches: Vec::new(),
        truncated: false,
        files_scanned: 0,
        files_skipped: 0,
    }
}

fn scan(
    checkout: &Path,
    start: &Path,
    needle: &str,
    extensions: &[&str],
    skip_docs: bool,
    limits: &ToolLimits,
) -> Result<SearchOutput, ToolError> {
    let checkout = checkout.canonicalize().map_err(|_| ToolError::NotFound)?;
    let mut scanner = Scanner {
        checkout: &checkout,
        needle,
        extensions,
        skip_docs,
        limits,
        output: empty_output(),
        stop: false,
    };
    if start.is_file() {
        scanner.consider_file(start);
    } else {
        scanner.visit_dir(start);
    }
    Ok(fit_json(
        scanner.output,
        limits.max_response_bytes,
        |output| {
            let dropped = output.matches.pop().is_some();
            if dropped {
                output.truncated = true;
            }
            dropped
        },
    ))
}

struct Scanner<'a> {
    checkout: &'a Path,
    needle: &'a str,
    extensions: &'a [&'a str],
    skip_docs: bool,
    limits: &'a ToolLimits,
    output: SearchOutput,
    stop: bool,
}

impl Scanner<'_> {
    fn visit_dir(&mut self, dir: &Path) {
        if self.stop {
            return;
        }
        let canonical = match dir.canonicalize() {
            Ok(path) if self.inside(path.as_path()) => path,
            _ => {
                self.output.files_skipped = self.output.files_skipped.saturating_add(1);
                self.output.truncated = true;
                return;
            }
        };
        let mut entries = match fs::read_dir(&canonical) {
            Ok(entries) => entries.flatten().collect::<Vec<_>>(),
            Err(_) => {
                self.output.files_skipped = self.output.files_skipped.saturating_add(1);
                return;
            }
        };
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if self.stop {
                return;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            let path = entry.path();
            let file_type = entry.file_type().ok();
            let is_symlink = file_type.as_ref().is_some_and(|kind| kind.is_symlink());
            let is_dir = file_type.as_ref().is_some_and(|kind| kind.is_dir())
                || (is_symlink && path.is_dir());
            if is_dir {
                if self.skip_docs && name == "docs" {
                    continue;
                }
                self.visit_dir(&path);
            } else {
                self.consider_file(&path);
            }
        }
    }

    fn consider_file(&mut self, path: &Path) {
        if self.stop || !has_extension(path, self.extensions) {
            return;
        }
        if self.output.files_scanned as usize >= self.limits.max_files_scanned {
            self.output.truncated = true;
            self.stop = true;
            return;
        }
        let Ok(metadata) = fs::metadata(path) else {
            self.output.files_skipped = self.output.files_skipped.saturating_add(1);
            return;
        };
        if !metadata.is_file() || metadata.len() > self.limits.max_file_bytes as u64 {
            self.output.files_skipped = self.output.files_skipped.saturating_add(1);
            return;
        }
        let Ok(canonical) = path.canonicalize() else {
            self.output.files_skipped = self.output.files_skipped.saturating_add(1);
            return;
        };
        if !self.inside(&canonical) {
            self.output.files_skipped = self.output.files_skipped.saturating_add(1);
            self.output.truncated = true;
            return;
        }
        self.output.files_scanned = self.output.files_scanned.saturating_add(1);
        let Ok(file) = File::open(&canonical) else {
            self.output.files_skipped = self.output.files_skipped.saturating_add(1);
            return;
        };
        let mut consumed = 0usize;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            if self.stop {
                return;
            }
            let Ok(line) = line else {
                self.output.files_skipped = self.output.files_skipped.saturating_add(1);
                return;
            };
            consumed = consumed.saturating_add(line.len());
            if line.contains('\0') || consumed > self.limits.max_file_bytes {
                self.output.files_skipped = self.output.files_skipped.saturating_add(1);
                return;
            }
            if !line.to_lowercase().contains(self.needle) {
                continue;
            }
            if self.output.matches.len() >= self.limits.max_matches {
                self.output.truncated = true;
                self.stop = true;
                return;
            }
            let Ok(rel) = relative_display(self.checkout, &canonical) else {
                self.output.files_skipped = self.output.files_skipped.saturating_add(1);
                return;
            };
            self.output.matches.push(SearchHit {
                path: rel,
                line: u32::try_from(index + 1).unwrap_or(u32::MAX),
                snippet: truncate_bytes(line.trim(), self.limits.max_snippet_bytes),
            });
        }
    }

    fn inside(&self, path: &Path) -> bool {
        path.starts_with(self.checkout)
    }
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext))
        })
}
