//! MCP prompt definitions for Muninn.
//!
//! Equivalent to `neo4j_agent_memory.mcp._prompts`.
//!
//! Prompts surface as structured workflows that guide the LLM through
//! memory-aware interactions. Organized into profiles:
//! - **Core**: `memory-conversation` (always available)
//! - **Extended**: `memory-reasoning`, `memory-review`

/// A single message in a prompt template.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptMessage {
    /// Role of the message: "user" or "assistant".
    pub role: String,
    /// Content of the message.
    pub content: String,
}

/// A named MCP prompt definition.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptDefinition {
    /// Prompt name (e.g., "memory-conversation").
    pub name: String,
    /// Description of what this prompt does.
    pub description: String,
    /// Arguments the prompt accepts (name, optional description).
    pub arguments: Vec<PromptArgument>,
    /// The messages that make up this prompt.
    pub messages: Vec<PromptMessage>,
}

/// An argument for a prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptArgument {
    /// Argument name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Default value (if any).
    pub default: Option<String>,
}

/// Profile determines which prompts are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptProfile {
    /// Only core prompts (memory-conversation).
    Core,
    /// All prompts including extended (default).
    #[default]
    Extended,
}

/// Returns the core prompt: `memory-conversation`.
///
/// Initializes a memory-aware conversation by loading context from memory
/// and instructing the LLM on how to use memory tools throughout the conversation.
pub fn memory_conversation(session_id: Option<&str>) -> PromptDefinition {
    let session_hint = session_id
        .map(|sid| format!(" for session '{}'", sid))
        .unwrap_or_default();

    let session_param = session_id
        .map(|sid| format!(" (session_id='{}')", sid))
        .unwrap_or_default();

    PromptDefinition {
        name: "memory-conversation".to_owned(),
        description: "Initialize a memory-aware conversation.".to_owned(),
        arguments: vec![PromptArgument {
            name: "session_id".to_owned(),
            description: Some("Optional session identifier".to_owned()),
            default: Some("".to_owned()),
        }],
        messages: vec![PromptMessage {
            role: "user".to_owned(),
            content: format!(
                "Start a memory-aware conversation{session_hint}.\n\n\
                 Steps:\n\
                 1. Call memory_get_context to load relevant memories{session_param}\n\
                 2. Review the loaded context for:\n\
                    - Previous conversation topics and decisions\n\
                    - Known user preferences\n\
                    - Relevant entities and relationships\n\
                 3. Greet the user, referencing relevant context if available\n\
                 4. Throughout the conversation:\n\
                    - Call memory_store_message for important user messages\n\
                    - Call memory_add_preference when preferences are expressed\n\
                    - Call memory_add_entity when new people/places/orgs are mentioned\n\
                    - Call memory_search if the user asks about past interactions"
            ),
        }],
    }
}

/// Returns the extended prompt: `memory-reasoning`.
///
/// Guides the LLM through structured reasoning with step-by-step trace recording.
pub fn memory_reasoning(task: &str) -> PromptDefinition {
    PromptDefinition {
        name: "memory-reasoning".to_owned(),
        description: "Record a reasoning trace for a complex task.".to_owned(),
        arguments: vec![PromptArgument {
            name: "task".to_owned(),
            description: Some("The task to solve and record reasoning for".to_owned()),
            default: None,
        }],
        messages: vec![PromptMessage {
            role: "user".to_owned(),
            content: format!(
                "Solve this task and record your reasoning: {task}\n\n\
                 Steps:\n\
                 1. Call memory_start_trace with the task description\n\
                 2. For each significant reasoning step:\n\
                    a. Think about what to do next (thought)\n\
                    b. Take an action or make a decision (action)\n\
                    c. Observe the result (observation)\n\
                    d. Call memory_record_step with thought, action, observation\n\
                    e. If you use a tool, include tool_name, tool_args, tool_result\n\
                 3. When the task is complete:\n\
                    - Call memory_complete_trace with the outcome\n\
                    - Set success=true if completed, false if not\n\
                 4. Summarize the reasoning process and final outcome"
            ),
        }],
    }
}

/// Returns the extended prompt: `memory-review`.
///
/// Reviews stored knowledge and flags contradictions.
pub fn memory_review() -> PromptDefinition {
    PromptDefinition {
        name: "memory-review".to_owned(),
        description: "Review stored knowledge and flag contradictions.".to_owned(),
        arguments: vec![],
        messages: vec![PromptMessage {
            role: "user".to_owned(),
            content: "\
                Review everything stored in my memory and provide a summary.\n\n\
                Steps:\n\
                1. Call memory_search with a broad query to find entities\n\
                2. Call memory_search with memory_types=['preferences'] to find preferences\n\
                3. Call memory_list_sessions to see conversation history\n\
                4. For the most relevant entities, call memory_get_entity for details\n\
                5. Compile a summary organized by:\n\
                   - Known entities (people, organizations, locations)\n\
                   - Stored preferences by category\n\
                   - Key facts and relationships\n\
                   - Recent conversation topics\n\
                6. Flag any potential contradictions or outdated information\n\
                7. Suggest any preferences or facts that should be updated"
                .to_owned(),
        }],
    }
}

/// Returns all prompts available for the given profile.
pub fn list_prompts(profile: PromptProfile) -> Vec<PromptDefinition> {
    let mut prompts = vec![memory_conversation(None)];

    if profile == PromptProfile::Extended {
        prompts.push(memory_reasoning("example task"));
        prompts.push(memory_review());
    }

    prompts
}

/// Returns a prompt by name, if it exists in the given profile.
pub fn get_prompt(name: &str, profile: PromptProfile) -> Option<PromptDefinition> {
    match name {
        "memory-conversation" => Some(memory_conversation(None)),
        "memory-reasoning" if profile == PromptProfile::Extended => {
            Some(memory_reasoning("example task"))
        }
        "memory-review" if profile == PromptProfile::Extended => Some(memory_review()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_conversation_has_correct_name() {
        let prompt = memory_conversation(None);
        assert_eq!(prompt.name, "memory-conversation");
        assert_eq!(prompt.messages.len(), 1);
        assert_eq!(prompt.messages[0].role, "user");
    }

    #[test]
    fn memory_conversation_includes_session_id() {
        let prompt = memory_conversation(Some("test-session-123"));
        assert!(prompt.messages[0].content.contains("test-session-123"));
    }

    #[test]
    fn memory_conversation_includes_steps() {
        let prompt = memory_conversation(None);
        let content = &prompt.messages[0].content;
        assert!(content.contains("memory_get_context"));
        assert!(content.contains("memory_store_message"));
        assert!(content.contains("memory_add_preference"));
        assert!(content.contains("memory_search"));
    }

    #[test]
    fn memory_reasoning_has_correct_name() {
        let prompt = memory_reasoning("solve the bug");
        assert_eq!(prompt.name, "memory-reasoning");
        assert!(prompt.messages[0].content.contains("solve the bug"));
    }

    #[test]
    fn memory_reasoning_includes_trace_steps() {
        let prompt = memory_reasoning("test");
        let content = &prompt.messages[0].content;
        assert!(content.contains("memory_start_trace"));
        assert!(content.contains("memory_record_step"));
        assert!(content.contains("memory_complete_trace"));
    }

    #[test]
    fn memory_review_has_no_arguments() {
        let prompt = memory_review();
        assert_eq!(prompt.name, "memory-review");
        assert!(prompt.arguments.is_empty());
    }

    #[test]
    fn memory_review_includes_review_steps() {
        let prompt = memory_review();
        let content = &prompt.messages[0].content;
        assert!(content.contains("memory_search"));
        assert!(content.contains("memory_list_sessions"));
        assert!(content.contains("memory_get_entity"));
    }

    #[test]
    fn list_prompts_core_returns_one() {
        let prompts = list_prompts(PromptProfile::Core);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "memory-conversation");
    }

    #[test]
    fn list_prompts_extended_returns_three() {
        let prompts = list_prompts(PromptProfile::Extended);
        assert_eq!(prompts.len(), 3);
        assert_eq!(prompts[0].name, "memory-conversation");
        assert_eq!(prompts[1].name, "memory-reasoning");
        assert_eq!(prompts[2].name, "memory-review");
    }

    #[test]
    fn get_prompt_finds_core_prompt() {
        let prompt = get_prompt("memory-conversation", PromptProfile::Core);
        assert!(prompt.is_some());
    }

    #[test]
    fn get_prompt_finds_extended_prompt() {
        let prompt = get_prompt("memory-reasoning", PromptProfile::Extended);
        assert!(prompt.is_some());
    }

    #[test]
    fn get_prompt_returns_none_for_core_extended_prompt() {
        let prompt = get_prompt("memory-reasoning", PromptProfile::Core);
        assert!(prompt.is_none());
    }

    #[test]
    fn get_prompt_returns_none_for_unknown() {
        let prompt = get_prompt("unknown-prompt", PromptProfile::Extended);
        assert!(prompt.is_none());
    }

    #[test]
    fn prompt_argument_has_fields() {
        let arg = PromptArgument {
            name: "session_id".to_owned(),
            description: Some("Session ID".to_owned()),
            default: Some("".to_owned()),
        };
        assert_eq!(arg.name, "session_id");
        assert_eq!(arg.description.as_deref(), Some("Session ID"));
        assert_eq!(arg.default.as_deref(), Some(""));
    }
}
