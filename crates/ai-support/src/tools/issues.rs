//! Search existing GitHub issues.
//!
//! The real client is a GitHub App held by this server. Until that exists the
//! tool returns [`IssueSearchStatus::NotConfigured`] and no hits, after the
//! same query-length check the other tools use. It does not call the network.

use serde::Serialize;

use super::{ToolError, prepare_query};
use crate::limits::ToolLimits;

/// One issue the model is allowed to cite. Unused while the tool is a stub;
/// the shape is part of the tool result so a later client does not change it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueHit {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSearchStatus {
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueSearchOutput {
    pub status: IssueSearchStatus,
    pub matches: Vec<IssueHit>,
    pub truncated: bool,
}

pub fn search_issues(query: &str, limits: &ToolLimits) -> Result<IssueSearchOutput, ToolError> {
    prepare_query(query, limits.max_query_chars)?;
    Ok(IssueSearchOutput {
        status: IssueSearchStatus::NotConfigured,
        matches: Vec::new(),
        truncated: false,
    })
}
