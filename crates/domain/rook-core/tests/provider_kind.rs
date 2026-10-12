use rook_core::ProviderKind;

#[test]
fn azure_openai_provider_kind_round_trips() {
    assert_eq!(
        ProviderKind::try_from("azure-openai").unwrap(),
        ProviderKind::AzureOpenAI
    );
    assert_eq!(ProviderKind::AzureOpenAI.as_str(), "azure-openai");
}
