//! Application output budget, shared by adapters.
pub const MAX_OUTPUT_BYTES: usize = 65_536;
pub fn bounded_text(text: String) -> String {
    bounded_text_with_limit(text, MAX_OUTPUT_BYTES)
}
pub(crate) fn bounded_text_with_limit(mut text: String, limit: usize) -> String {
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[output truncated]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_limits_preserve_utf8_and_signal_lost_content() {
        for (limit, prefix) in [(0, ""), (1, ""), (2, "é"), (5, "é"), (6, "é🦀"), (7, "é🦀")]
        {
            assert_eq!(
                bounded_text_with_limit("é🦀é".into(), limit),
                format!("{prefix}\n[output truncated]")
            );
        }
        assert_eq!(bounded_text_with_limit("é🦀é".into(), 8), "é🦀é");
    }

    #[test]
    fn ordinary_output_keeps_the_existing_content_plus_marker_contract() {
        let full = "é".repeat(MAX_OUTPUT_BYTES / 2);
        assert_eq!(bounded_text(full.clone()), full);
        assert_eq!(
            bounded_text(format!("{full}é")),
            format!("{full}\n[output truncated]")
        );
    }
}
