use muninn_mcp::{get_prompt, PromptProfile};

#[test]
fn get_prompt_finds_memory_review_in_extended_profile() {
    let prompt = get_prompt("memory-review", PromptProfile::Extended)
        .expect("memory-review is available in the extended profile");
    assert_eq!(prompt.name, "memory-review");
    assert!(prompt.arguments.is_empty());
    assert!(prompt.messages[0].content.contains("memory_list_sessions"));
}

#[test]
fn get_prompt_hides_memory_review_in_core_profile() {
    assert!(get_prompt("memory-review", PromptProfile::Core).is_none());
}
