use huginn::config::{resolve_with, Config};
use huginn_presentation::settings::UiConfig;
use std::path::PathBuf;

#[test]
fn resolution_prefers_an_explicit_path_over_the_default() {
    let root = tempfile::tempdir().unwrap();
    let primary = root.path().join("huginn/config.toml");
    let explicit = root.path().join("elsewhere.toml");

    // No explicit path: the caller uses the default.
    assert_eq!(resolve_with(None, &primary), primary);

    // An explicit path never falls back and never reads the default.
    assert_eq!(resolve_with(Some(&explicit), &primary), explicit);
}

#[test]
fn default_paths_live_under_the_cortex_home() {
    let config_path = Config::primary_config_path();
    assert!(
        config_path.ends_with(PathBuf::from(".cortex").join("huginn").join("config.toml")),
        "unexpected default config path: {}",
        config_path.display()
    );
    assert_eq!(Config::resolve_path(None), Config::primary_config_path());

    let database = Config::default().db;
    assert!(
        database.ends_with(PathBuf::from(".cortex").join("huginn").join("sessions.db")),
        "unexpected default database path: {}",
        database.display()
    );
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
