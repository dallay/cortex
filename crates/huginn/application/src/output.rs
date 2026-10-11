//! Application output budget, shared by adapters.
pub const MAX_OUTPUT_BYTES: usize = 65_536;
pub fn bounded_text(mut text: String) -> String {
    if text.len() > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[output truncated]");
    }
    text
}
