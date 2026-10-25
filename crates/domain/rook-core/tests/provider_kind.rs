use rook_core::ProviderKind;

#[test]
fn openai_compatible_provider_kind_round_trips() {
    assert_eq!(
        ProviderKind::try_from("openai-compatible").unwrap(),
        ProviderKind::OpenAiCompatible
    );
    assert_eq!(ProviderKind::OpenAiCompatible.as_str(), "openai-compatible");
}

#[test]
fn azure_openai_provider_kind_round_trips() {
    assert_eq!(
        ProviderKind::try_from("azure-openai").unwrap(),
        ProviderKind::AzureOpenAI
    );
    assert_eq!(ProviderKind::AzureOpenAI.as_str(), "azure-openai");
}
