//! Read a window of a text file inside the checkout.
//!
//! `offset` is the 1-based line to start at. `limit` is how many lines to
//! return, already clamped to the server cap. The file is not loaded whole.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;

use super::sandbox::{relative_display, resolve_inside};
use super::{ToolError, fit_json};
use crate::limits::{ToolLimits, truncate_bytes};

/// Arguments for [`read_file`]. Both ends of the window are server-clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileRequest {
    pub path: String,
    pub offset: u32,
    pub limit: u32,
}

/// A line window. `eof` is false when the cap stopped the read before the
/// file ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileWindow {
    pub path: String,
    pub offset: u32,
    pub lines: Vec<String>,
    pub truncated: bool,
    pub eof: bool,
}

pub fn read_file(
    checkout: &Path,
    request: &ReadFileRequest,
    limits: &ToolLimits,
) -> Result<FileWindow, ToolError> {
    if request.offset == 0 || request.limit == 0 {
        return Err(ToolError::InvalidRange);
    }
    let path = resolve_inside(checkout, &request.path)?;
    if !path.is_file() {
        return Err(ToolError::NotAFile);
    }
    let display = relative_display(checkout, &path)?;
    let file = File::open(&path).map_err(|_| ToolError::NotFound)?;
    let mut lines = Vec::new();
    let mut bytes = 0usize;
    let mut consumed = 0usize;
    let mut truncated = false;
    let mut exhausted = true;
    let line_limit = request.limit.min(limits.max_read_lines);
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|_| ToolError::Binary)?;
        consumed = consumed.saturating_add(line.len());
        if line.contains('\0') {
            return Err(ToolError::Binary);
        }
        if consumed > limits.max_file_bytes {
            truncated = true;
            exhausted = false;
            break;
        }
        let line_no = u32::try_from(index + 1).unwrap_or(u32::MAX);
        if line_no < request.offset {
            continue;
        }
        if lines.len() as u32 >= line_limit {
            truncated = true;
            exhausted = false;
            break;
        }
        let snippet = truncate_bytes(&line, limits.max_snippet_bytes);
        if bytes.saturating_add(snippet.len()) > limits.max_read_bytes {
            truncated = true;
            exhausted = false;
            break;
        }
        if snippet.len() < line.len() {
            truncated = true;
        }
        bytes = bytes.saturating_add(snippet.len());
        lines.push(snippet);
    }
    Ok(fit_json(
        FileWindow {
            path: display,
            offset: request.offset,
            lines,
            truncated,
            eof: exhausted,
        },
        limits.max_response_bytes,
        |window| {
            let dropped = window.lines.pop().is_some();
            if dropped {
                window.truncated = true;
                window.eof = false;
            }
            dropped
        },
    ))
}
