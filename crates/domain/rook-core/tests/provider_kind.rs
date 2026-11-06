use rook_core::{descriptor_by_id, descriptor_for};

#[test]
fn openai_compatible_descriptor_resolves() {
    let descriptor = descriptor_by_id("openai-compatible").expect("declared descriptor");
    assert_eq!(descriptor.id, "openai-compatible");
    assert_eq!(
        descriptor_for("openai-compatible").map(|d| d.id),
        Some(descriptor.id)
    );
}

#[test]
fn azure_openai_descriptor_resolves() {
    let descriptor = descriptor_by_id("azure-openai").expect("declared descriptor");
    assert_eq!(descriptor.id, "azure-openai");
    assert_eq!(
        descriptor_for("azure-openai").map(|d| d.id),
        Some(descriptor.id)
    );
    // The pre-migration serde spelling stays readable through the alias map.
    assert_eq!(
        descriptor_for("azureopenai").map(|d| d.id),
        Some(descriptor.id)
    );
}
