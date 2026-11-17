//! Valores e invariantes independientes de HTTP y Neo4j.

pub fn valid_role(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "system" | "tool")
}

pub struct NewMessage {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl NewMessage {
    pub fn is_valid(&self) -> bool {
        !self.session_id.is_empty() && self.session_id.len() <= 512 && valid_role(&self.role)
    }
}

pub struct Message {
    pub id: uuid::Uuid,
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
}

pub struct Conversation {
    pub id: uuid::Uuid,
    pub session_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<Message>,
}

pub struct SessionInfo {
    pub session_id: String,
    pub message_count: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::valid_role;

    #[test]
    fn accepts_the_four_pinned_reference_roles() {
        for role in ["user", "assistant", "system", "tool"] {
            assert!(valid_role(role));
        }
        assert!(!valid_role("administrator"));
    }
}
