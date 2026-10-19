//! Caps for read-only tools.
//!
//! The numbers belong to the server. A result that does not fit is truncated.
//! Nothing in this crate shells out to get a bigger one.

/// How much of a checkout one tool call may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolLimits {
    pub max_query_chars: usize,
    pub max_matches: usize,
    pub max_snippet_bytes: usize,
    pub max_response_bytes: usize,
    pub max_read_lines: u32,
    pub max_read_bytes: usize,
    pub max_files_scanned: usize,
    pub max_file_bytes: usize,
}

impl ToolLimits {
    pub const DEFAULT: Self = Self {
        max_query_chars: 200,
        max_matches: 20,
        max_snippet_bytes: 240,
        max_response_bytes: 12 * 1024,
        max_read_lines: 200,
        max_read_bytes: 12 * 1024,
        max_files_scanned: 2_000,
        max_file_bytes: 512 * 1024,
    };
}

impl Default for ToolLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Cut `text` to at most `max_bytes` on a char boundary. Over-long input is
/// marked with an ASCII ellipsis so the byte cap still holds.
pub(crate) fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let ellipsis = b"...";
    let budget = max_bytes.saturating_sub(ellipsis.len());
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_string();
    out.push_str("...");
    out
}
