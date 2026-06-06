// providers-core — shared utilities for provider adapters
//
// Design principles:
// - ZERO external dependencies
// - Non-inline functions (used by multiple providers)
// - All role strings are &'static str (no allocation)

pub mod request;
pub mod role;
pub mod sanitize;
pub mod sse;
pub mod validation;

pub use role::{Role, RoleExt, role_to_string};
pub use sanitize::{char_safe_truncate, sanitize_body};
pub use sse::{parse_event_text, process_bytes, SseEvent};
pub use validation::validate_response;
