//! Complete-turn boundaries over conversation history.

use crate::{Message, Role};

/// Exclusive end index of every complete turn within `messages[from..]`.
///
/// A turn is complete at a tool-call-free assistant message whose window
/// `messages[from..=index]` contains at least one user message. The returned `index + 1` is
/// a position history may be cut at: the cut keeps the whole turn, and an assistant message
/// that carries tool calls never ends a turn.
///
/// Compaction stops at these boundaries. `from` is the position an earlier summary already
/// covers, normally [`Session::summary_through`](crate::Session::summary_through), so a cut
/// neither re-summarizes nor discards covered history.
///
/// # Boundaries
///
/// - Tool-call-bearing assistants are skipped; only turn ends are returned. In a well-formed
///   history the results of a tool call follow the assistant that requested them, so a cut at
///   one of these ends keeps the pair. This walk does not check pairing: a tool-call-free
///   assistant placed between a call and its result still ends a turn, and a cut there would
///   split the pair. Callers that accept arbitrary message order have to keep call and result
///   together themselves.
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
