// role — shared Role → string mapping for OpenAI-compatible provider APIs

use rook_core::Role;

/// Map a `Role` to the wire-format string expected by OpenAI-compatible APIs.
///
/// Duplicated verbatim in providers-openai, providers-groq, and providers-ollama.
/// Centralized here to eliminate the duplication.
#[inline]
pub fn role_to_string(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Developer => "developer",
    }
}