//! Redaction at the terminal observation boundary.

/// Replacement text for a value that must not enter the conversation.
const REDACTED: &str = "[REDACTED]";
/// Assignment prefixes whose values are secret-shaped.
const KEY_MARKERS: [&str; 8] = [
    "password=",
    "passwd=",
    "secret=",
    "token=",
    "api_key=",
    "apikey=",
    "access_token=",
    "refresh_token=",
];

/// Replaces secret-shaped values before an observation reaches the model or an observer.
pub(super) fn redact_observation(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut redact_next = false;
    for part in text.split_inclusive(|character: char| character.is_whitespace()) {
        let token = part.trim_end_matches(char::is_whitespace);
        let whitespace = if token.is_empty() {
            part
        } else {
            part.trim_start_matches(token)
        };
        let lower = token.to_ascii_lowercase();
        if redact_next && !token.is_empty() {
            output.push_str(REDACTED);
            redact_next = false;
        } else if lower == "bearer" || lower == "basic" {
            output.push_str(token);
            redact_next = true;
        } else if let Some(marker) = KEY_MARKERS
            .iter()
            .find(|marker| lower.starts_with(**marker))
        {
            output.push_str(marker);
            output.push_str(REDACTED);
        } else if lower.starts_with("sk-")
            || lower.starts_with("ghp_")
            || lower.starts_with("github_pat_")
            || lower.starts_with("xoxb-")
            || lower.starts_with("akia")
            || lower.starts_with("-----begin")
        {
            output.push_str(REDACTED);
        } else {
            output.push_str(token);
        }
        output.push_str(whitespace);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::redact_observation;

    #[test]
    fn sim_secret_shaped_observations_are_redacted() {
        let shown = redact_observation(
            "stdout:\nTOKEN=abc123 Bearer bearer-value\nnormal\nsecret=keep-private\n",
        );
        assert!(!shown.contains("abc123"), "key value is gone: {shown}");
        assert!(
            !shown.contains("bearer-value"),
            "bearer value is gone: {shown}"
        );
        assert!(
            !shown.contains("keep-private"),
            "secret value is gone: {shown}"
        );
        assert!(
            shown.contains("normal"),
            "non-secret output remains: {shown}"
        );
    }
}
