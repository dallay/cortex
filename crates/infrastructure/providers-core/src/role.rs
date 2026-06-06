// role.rs — Role enum for provider message roles

use rook_core::Role as CoreRole;
use serde::{Deserialize, Serialize};

/// Role in a conversation message.
///
/// Mirrors the Role type from rook-core but defined here to avoid
/// a dependency on rook-core (which has many dependencies).
/// Providers use this to convert Role → API-specific strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    /// Convert Role to the canonical string used by OpenAI-compatible APIs.
    /// Returns a static str — no allocation.
    pub fn to_role_string(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// Convert `rook_core::Role` to the wire-format string for OpenAI-compatible APIs.
///
/// This is the helper that all providers should use instead of inline match blocks.
/// Handles the `Developer` role by mapping it to `"developer"` (not `"system"`),
/// since not all providers map Developer → System.
pub fn role_to_string(role: CoreRole) -> &'static str {
    match role {
        CoreRole::System => "system",
        CoreRole::User => "user",
        CoreRole::Assistant => "assistant",
        CoreRole::Developer => "developer",
    }
}

/// Extension trait for converting external role representations to Role.
pub trait RoleExt {
    fn to_role(&self) -> Option<Role>;
}

impl RoleExt for str {
    fn to_role(&self) -> Option<Role> {
        match self.to_lowercase().as_str() {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            "developer" => Some(Role::System), // Map developer to system
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_to_role_string() {
        assert_eq!(Role::System.to_role_string(), "system");
        assert_eq!(Role::User.to_role_string(), "user");
        assert_eq!(Role::Assistant.to_role_string(), "assistant");
        assert_eq!(Role::Tool.to_role_string(), "tool");
    }

    #[test]
    fn test_role_to_string_core_role() {
        // role_to_string takes CoreRole (from rook_core), not the local Role enum
        assert_eq!(role_to_string(CoreRole::System), "system");
        assert_eq!(role_to_string(CoreRole::User), "user");
        assert_eq!(role_to_string(CoreRole::Assistant), "assistant");
        assert_eq!(role_to_string(CoreRole::Developer), "developer");
    }

    #[test]
    fn test_role_to_role_string_is_static() {
        // Ensure no allocation by checking pointer stability
        let s1 = Role::User.to_role_string();
        let s2 = Role::User.to_role_string();
        assert_eq!(s1.as_ptr(), s2.as_ptr());
    }

    #[test]
    fn test_role_ext_to_role() {
        assert_eq!("system".to_role(), Some(Role::System));
        assert_eq!("USER".to_role(), Some(Role::User));
        assert_eq!("Assistant".to_role(), Some(Role::Assistant));
        assert_eq!("tool".to_role(), Some(Role::Tool));
        assert_eq!("developer".to_role(), Some(Role::System)); // Maps to System
        assert_eq!("unknown".to_role(), None);
    }

    #[test]
    fn test_role_copy() {
        let r = Role::User;
        let r2 = r;
        assert_eq!(r, r2);
    }

    #[test]
    fn test_role_serde() {
        let json = serde_json::to_string(&Role::Assistant).unwrap();
        assert_eq!(json, "\"assistant\"");
        let deserialized: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, Role::Assistant);
    }
}
