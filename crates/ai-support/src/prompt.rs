//! System prompt for the support model.
//!
//! The text is compiled into the binary. The APK cannot replace it, and a
//! checkout of the user's version is not a substitute for it: the prompt is
//! the server's policy, the checkout is the evidence.

/// Stable id returned by `POST /api/v1/assistant/chat`. Bump when the rules
/// in `prompts/system.md` change, and keep the `prompt-id` marker in that file
/// equal to this string.
pub const SYSTEM_PROMPT_ID: &str = "ai-support-draft-1";

const SYSTEM_PROMPT: &str = include_str!("../prompts/system.md");

pub fn system_prompt() -> &'static str {
    SYSTEM_PROMPT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_id_matches_the_file_and_the_rules_stay_explicit() {
        let text = system_prompt();
        assert!(text.contains(&format!("prompt-id: {SYSTEM_PROMPT_ID}")));
        assert!(text.contains("Only `routed` is a measurement."));
        assert!(text.contains("current_success"));
        assert!(text.contains("Do not say \"protection is working\""));
        assert!(text.contains("You have no shell."));
        assert!(text.contains("Do not invent issue numbers"));
        assert!(text.contains("not git history"));
        assert!(text.contains("hidden_selinux"));
        assert!(text.contains("runtime_checkable"));
        assert!(text.contains("not_configured"));
        assert!(text.contains("does not change settings"));
    }
}
