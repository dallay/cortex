use huginn::config::{resolve_with, Config};
use huginn_presentation::settings::UiConfig;

#[test]
fn resolution_prefers_explicit_then_primary_then_legacy() {
    let root = tempfile::tempdir().unwrap();
    let primary = root.path().join("huginn/config.toml");
    let legacy = root.path().join("agent/config.toml");
    std::fs::create_dir_all(primary.parent().unwrap()).unwrap();
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();

    // No file exists: writers create the primary path.
    assert_eq!(resolve_with(None, &primary, &legacy), primary);

    // Only the legacy file exists: readers and writers use it.
    std::fs::write(&legacy, "[provider]\n").unwrap();
    assert_eq!(resolve_with(None, &primary, &legacy), legacy);

    // The primary file wins once it exists.
    std::fs::write(&primary, "[ui]\n").unwrap();
    assert_eq!(resolve_with(None, &primary, &legacy), primary);

    // An explicit path never falls back.
    let explicit = root.path().join("elsewhere.toml");
    assert_eq!(resolve_with(Some(&explicit), &primary, &legacy), explicit);
}

#[test]
fn legacy_config_seeds_the_primary_file_and_stays_untouched() {
    let root = tempfile::tempdir().unwrap();
    let primary = root.path().join("huginn/config.toml");
    let legacy = root.path().join("agent/config.toml");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(
        &legacy,
        "provider = \"openai\"\nmodel = \"gpt-4o\"\n\n[ui.theme]\nname = \"dracula\"\n",
    )
    .unwrap();
    let legacy_bytes = std::fs::read_to_string(&legacy).unwrap();

    // Mirrors `tui_chat`: read through the resolver, write to the primary,
    // seed from whatever the process actually loaded.
    let read_path = resolve_with(None, &primary, &legacy);
    assert_eq!(read_path, legacy);
    let loaded = Config::load(Some(&read_path)).unwrap();
    huginn::ui_settings::persist_seeded(&loaded.ui, &primary, Some(&read_path)).unwrap();

    // The deprecated file is never modified.
    assert_eq!(std::fs::read_to_string(&legacy).unwrap(), legacy_bytes);
    // The new file keeps the whole configuration, not only `[ui]`.
    let reloaded = Config::load(Some(&primary)).unwrap();
    assert_eq!(reloaded.provider, "openai");
    assert_eq!(reloaded.model.as_deref(), Some("gpt-4o"));
    assert_eq!(reloaded.ui, loaded.ui);
    // The next start reads the primary path and stops warning.
    assert_eq!(resolve_with(None, &primary, &legacy), primary);
}

#[test]
fn ui_defaults_fill_absent_sections_and_partial_nested_tables() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    for (text, expected) in [
        ("", UiConfig::defaults()),
        ("[ui]\n", UiConfig::defaults()),
        ("[ui.theme]\nname = 'dracula'\n", {
            let mut expected = UiConfig::defaults();
            expected.theme.name = "dracula".into();
            expected
        }),
        ("[ui.indicators]\nscrollbar = false\n", {
            let mut expected = UiConfig::defaults();
            expected.indicators.scrollbar = false;
            expected
        }),
        ("[ui.toasts]\nlevel = 'warn'\n", {
            let mut expected = UiConfig::defaults();
            expected.toasts.level = "warn".into();
            expected
        }),
    ] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            Config::load(Some(&path)).unwrap().ui,
            expected,
            "input: {text:?}"
        );
    }
}
