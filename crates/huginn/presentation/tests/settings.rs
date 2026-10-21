use huginn_presentation::settings::*;

#[test]
fn defaults_round_trip_and_match_canonical() {
    let cfg = UiConfig::defaults();
    let theme = THEME_NAMES[0];
    assert_eq!(theme, "one-dark");
    assert_eq!(cfg.theme.name, "one-dark");
    assert!(!cfg.sound.bell);
    assert_eq!(cfg.toasts.level, "info");
    assert_eq!(cfg.toasts.duration_secs, 3);
    assert!(cfg.indicators.scrollbar);
    assert!(cfg.indicators.breadcrumbs);
    assert!(cfg.indicators.line_numbers);
    assert!(cfg.integrations.entries.is_empty());
}

#[test]
fn tab_cycling_wraps() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    assert_eq!(d.active_tab, 0);
    for _ in 0..TAB_COUNT {
        d.next_tab();
    }
    assert_eq!(d.active_tab, 0);
    d.prev_tab();
    assert_eq!(d.active_tab, TAB_COUNT - 1);
}

#[test]
fn item_selection_wraps() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.active_tab = TAB_THEME;
    let count = THEME_NAMES.len();
    d.select_next();
    assert_eq!(d.selections[TAB_THEME].selected, 1);
    d.select_prev();
    assert_eq!(d.selections[TAB_THEME].selected, 0);
    d.select_prev();
    assert_eq!(d.selections[TAB_THEME].selected, count - 1);
}

#[test]
fn toggle_advances_toast_level_cycle() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.active_tab = TAB_TOASTS;
    assert_eq!(d.working.toasts.level, 0);
    d.toggle_selected();
    assert_eq!(d.working.toasts.level, 1);
    d.toggle_selected();
    assert_eq!(d.working.toasts.level, 2);
    d.toggle_selected();
    assert_eq!(d.working.toasts.level, 0);
}

#[test]
fn toggle_advances_toast_duration_cycle() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.active_tab = TAB_TOASTS;
    d.select_next();
    assert_eq!(d.working.toasts.duration, 0);
    d.toggle_selected();
    assert_eq!(d.working.toasts.duration, 1);
    d.toggle_selected();
    assert_eq!(d.working.toasts.duration, 2);
    d.toggle_selected();
    assert_eq!(d.working.toasts.duration, 3);
    d.toggle_selected();
    assert_eq!(d.working.toasts.duration, 0);
}

#[test]
fn projected_ui_config_matches_working_copy() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.active_tab = TAB_INDICATORS;
    d.toggle_selected(); // scrollbar off
    d.select_next();
    d.toggle_selected(); // breadcrumbs off, line_numbers unchanged
    d.active_tab = TAB_TOASTS;
    d.toggle_selected();
    d.toggle_selected();
    let projected = d.projected_ui_config();
    assert!(!projected.indicators.scrollbar);
    assert!(!projected.indicators.breadcrumbs);
    assert!(projected.indicators.line_numbers);
    assert_eq!(projected.toasts.level, "error");
}

#[test]
fn apply_invokes_closure_exactly_once() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    let calls = std::sync::Mutex::new(Vec::<UiConfig>::new());
    let mut save = |ui: &UiConfig| -> anyhow::Result<()> {
        calls.lock().unwrap().push(ui.clone());
        Ok(())
    };
    d.apply(&mut save).unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(calls.lock().unwrap()[0], UiConfig::defaults());
}

#[test]
fn apply_failure_records_error_and_keeps_working_copy() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    let mut save = |_: &UiConfig| -> anyhow::Result<()> { Err(anyhow::anyhow!("disk full")) };
    let err = d.apply(&mut save).unwrap_err();
    assert!(matches!(err, ApplyError::Persist(ref m) if m.contains("disk full")));
    assert!(d.last_error.is_some());
    assert_eq!(d.working.theme.name_index, 0);
}

#[test]
fn discard_does_not_invoke_closure() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.discard();
    assert!(d.last_error.is_none());
}

#[test]
fn add_provider_only_on_integrations_row_zero() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    assert!(!d.handle_add_provider());
    d.active_tab = TAB_INTEGRATIONS;
    assert!(d.handle_add_provider());
}

#[test]
fn projected_config_preserves_untouched_unknown_values() {
    let mut cfg = UiConfig::defaults();
    cfg.theme.name = "my-custom".into();
    cfg.toasts.level = "debug".into();
    cfg.toasts.duration_secs = 7;
    cfg.integrations.entries = vec!["github".into(), "linear".into()];

    // Opening the dialog and pressing Enter without touching a row must
    // never rewrite values the dialog cannot represent.
    let d = SettingsDialog::new(&cfg);
    assert_eq!(d.projected_ui_config(), cfg);
}

#[test]
fn touching_a_cycle_overrides_only_that_value() {
    let mut cfg = UiConfig::defaults();
    cfg.toasts.duration_secs = 7;
    cfg.integrations.entries = vec!["github".into()];
    let mut d = SettingsDialog::new(&cfg);

    d.active_tab = TAB_THEME;
    d.select_next();
    d.toggle_selected();

    d.active_tab = TAB_TOASTS;
    d.toggle_selected();

    let projected = d.projected_ui_config();
    assert_eq!(projected.theme.name, "one-light");
    assert_eq!(projected.toasts.level, "warn");
    // Untouched duration and entries keep their persisted values.
    assert_eq!(projected.toasts.duration_secs, 7);
    assert_eq!(projected.integrations.entries, vec!["github".to_string()]);
}

#[test]
fn snapshot_items_match_locked_d6() {
    let cfg = UiConfig::defaults();
    let mut d = SettingsDialog::new(&cfg);
    d.active_tab = TAB_THEME;
    assert_eq!(d.snapshot().items.len(), THEME_NAMES.len());
    d.active_tab = TAB_INDICATORS;
    assert_eq!(d.snapshot().items.len(), 3);
    d.active_tab = TAB_SOUND;
    assert_eq!(d.snapshot().items.len(), 1);
    d.active_tab = TAB_TOASTS;
    assert_eq!(d.snapshot().items.len(), 2);
    d.active_tab = TAB_INTEGRATIONS;
    assert_eq!(d.snapshot().items.len(), 1);
}
