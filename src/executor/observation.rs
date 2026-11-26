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
/// Assignment markers whose value is the next whitespace-delimited field.
const COLON_MARKERS: [&str; 11] = [
    "password:",
    "passwd:",
    "secret:",
    "token:",
    "api_key:",
    "apikey:",
    "access_token:",
    "refresh_token:",
    "key:",
    "\"key\":",
    "\"token\":",
];

/// The combined stdout and stderr bound passed by the terminal session.
const OBSERVATION_BYTES: usize = 8192;

/// Replaces exact values first, then secret-shaped values, before an observation reaches the model
/// or an observer. The input is bounded to [`OBSERVATION_BYTES`], so each exact-value pass is bounded
/// by the captured observation size and never reads an unbounded stream.
pub(super) fn redact_observation(text: &str, secrets: &[String]) -> String {
    debug_assert!(
        text.len() <= OBSERVATION_BYTES.saturating_add(64),
        "terminal observations stay within their bounded streams"
    );
    let mut output = text.to_owned();
    for secret in secrets {
        if secret.len() < 8 {
            continue;
        }
        output = output.replace(secret, REDACTED);
    }
    let mut marked = String::with_capacity(output.len());
    let mut redact_next = false;
    for part in output.split_inclusive(|character: char| character.is_whitespace()) {
        let token = part.trim_end_matches(char::is_whitespace);
        let whitespace = if token.is_empty() {
            part
        } else {
            part.trim_start_matches(token)
        };
        let lower = token.to_ascii_lowercase();
        if redact_next && !token.is_empty() {
            marked.push_str(REDACTED);
            redact_next = false;
        } else if lower == "bearer" || lower == "basic" {
            marked.push_str(token);
            redact_next = true;
        } else if let Some(marker) = KEY_MARKERS
            .iter()
            .find(|marker| lower.starts_with(**marker))
        {
            marked.push_str(marker);
            marked.push_str(REDACTED);
        } else if COLON_MARKERS.iter().any(|marker| lower.ends_with(*marker)) {
            marked.push_str(token);
            redact_next = !token.is_empty();
        } else if lower.starts_with("sk-")
            || lower.starts_with("ghp_")
            || lower.starts_with("github_pat_")
            || lower.starts_with("xoxb-")
            || lower.starts_with("akia")
            || lower.starts_with("-----begin")
        {
            marked.push_str(REDACTED);
        } else {
            marked.push_str(token);
        }
        marked.push_str(whitespace);
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::redact_observation;
    use lgwks_std::seeded::Seeded;

    #[test]
    fn sim_secret_shaped_observations_are_redacted() {
        let shown = redact_observation(
            "stdout:\nTOKEN=abc123 Bearer bearer-value\nnormal\nsecret=keep-private\n",
            &[],
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

    #[test]
    fn sim_colon_markers_redact_without_exact_secret_knowledge() {
        let shown = redact_observation("KEY: eight888 {\"token\": \"eight999\"} safe text", &[]);
        assert!(!shown.contains("eight888"), "colon value is gone: {shown}");
        assert!(!shown.contains("eight999"), "JSON value is gone: {shown}");
        assert!(shown.contains("safe text"), "safe text remains: {shown}");
        let eight = String::from("12345678");
        assert_eq!(
            redact_observation("value 12345678", &[eight]),
            "value [REDACTED]"
        );
    }

    #[test]
    fn sim_exact_secrets_at_seeded_offsets_and_forms_never_survive()
    -> Result<(), Box<dyn std::error::Error>> {
        for seed in 0..256_u64 {
            let mut rng = Seeded::from_seed(seed);
            let secret = format!("secret-{seed:016x}");
            let offset = usize::try_from(rng.below(32)?)?;
            let prefix = "n".repeat(offset);
            let suffix = "safe text";
            for form in [
                format!("{prefix}KEY: {secret} {suffix}"),
                format!(r#"{prefix}{{"token": "{secret}"}} {suffix}"#),
                format!("{prefix}Bearer {secret} {suffix}"),
            ] {
                let shown = redact_observation(&form, std::slice::from_ref(&secret));
                assert!(!shown.contains(&secret), "seed {seed}: {shown}");
                assert!(
                    shown.contains(suffix),
                    "seed {seed}: non-secret text was lost"
                );
            }
        }
        Ok(())
    }
}
