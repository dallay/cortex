//! Complete-turn boundaries over conversation history.

use crate::{Message, Role};

/// Exclusive end index of every complete turn within `messages[from..]`.
///
/// A turn is complete at a tool-call-free assistant message whose window
/// `messages[from..=index]` contains at least one user message. The returned `index + 1` is
/// a position history may be cut at: the cut keeps the whole turn and never splits a
/// tool-call/result pair, because an assistant message that carries tool calls never ends a
/// turn.
///
/// Compaction stops at these boundaries. `from` is the position an earlier summary already
/// covers, normally [`Session::summary_through`](crate::Session::summary_through), so a cut
/// neither re-summarizes nor discards covered history.
///
/// # Boundaries
///
/// - Tool-call-bearing assistants are skipped; only turn ends are returned.
/// - A window with no user message emits nothing: an assistant answering a request from
///   before `from` cannot close a turn inside the window.
/// - `from` at or past the end of `messages` is safe and yields an empty vector.
/// - Ends are strictly increasing, and the walk is a single O(n) pass that tracks only the
///   most recent user index seen at or after `from`.
pub fn complete_turn_ends(messages: &[Message], from: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut last_user: Option<usize> = None;
    for (index, message) in messages.iter().enumerate().skip(from) {
        match message.role {
            Role::User => last_user = Some(index),
            Role::Assistant if message.tool_calls.is_empty() && last_user.is_some() => {
                ends.push(index + 1);
            }
            _ => {}
        }
    }
    ends
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;
    use serde_json::json;

    fn assistant_text(content: &str) -> Message {
        Message::text(Role::Assistant, content)
    }

    fn assistant_with_tool_call(id: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: "read".into(),
                arguments: json!({}),
            }],
            tool_call_id: None,
        }
    }

    #[test]
    fn empty_input_returns_no_ends() {
        assert!(complete_turn_ends(&[], 0).is_empty());
    }

    #[test]
    fn single_user_only_session_emits_nothing() {
        let messages = vec![Message::text(Role::User, "hi")];
        assert!(complete_turn_ends(&messages, 0).is_empty());
    }

    #[test]
    fn complete_turn_with_user_emits_end_after_assistant() {
        let messages = vec![
            Message::text(Role::User, "request"),
            assistant_text("response"),
        ];
        assert_eq!(complete_turn_ends(&messages, 0), vec![2]);
    }

    #[test]
    fn tool_calling_assistant_is_skipped() {
        let messages = vec![
            Message::text(Role::User, "request"),
            assistant_with_tool_call("call-1"),
            Message::tool("call-1".to_string(), "result".into()),
            assistant_text("final"),
        ];
        // The tool-call assistant at index 1 must be ignored, so the only end is the final
        // assistant at index 3, reported one past itself.
        assert_eq!(complete_turn_ends(&messages, 0), vec![4]);
    }

    #[test]
    fn boundary_at_from_without_user_emits_nothing() {
        // from=2 begins after every user message, so the only assistant left has no user in
        // its window and must be skipped.
        let messages = vec![
            Message::text(Role::User, "ignored"),
            assistant_text("pre-from"),
            Message::text(Role::System, "tool result marker"),
            assistant_text("post-from"),
        ];
        assert!(complete_turn_ends(&messages, 2).is_empty());
    }

    #[test]
    fn from_past_end_is_safe() {
        let messages = vec![Message::text(Role::User, "hi")];
        assert!(complete_turn_ends(&messages, 5).is_empty());
    }

    #[test]
    fn long_alternating_session_emits_one_end_per_complete_turn() {
        // Synthetic 5,000-message session: assistant messages with no user in their window,
        // then turns alternating user / tool-call-free assistant with interleaved tool-call
        // pairs. Expected ends are recorded while building the fixture, so the assertion
        // states the intended boundaries instead of restating the rule being tested.
        const TURNS: usize = 1_250;
        const PREFIX_ASSISTANTS: usize = 1_666;
        let mut messages: Vec<Message> = Vec::with_capacity(5_000);
        for index in 0..PREFIX_ASSISTANTS {
            messages.push(assistant_text(&format!("prefix {index}")));
        }

        let mut expected: Vec<usize> = Vec::with_capacity(TURNS);
        for turn in 0..TURNS {
            messages.push(Message::text(Role::User, format!("request {turn}")));
            if turn % 3 == 0 {
                messages.push(assistant_with_tool_call(&format!("call-{turn}")));
                messages.push(Message::tool(
                    format!("call-{turn}"),
                    format!("result {turn}"),
                ));
            }
            messages.push(assistant_text(&format!("done {turn}")));
            expected.push(messages.len());
        }

        assert_eq!(messages.len(), 5_000);
        assert_eq!(complete_turn_ends(&messages, 0), expected);
    }
}
