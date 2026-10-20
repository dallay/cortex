//! Host-owned settings modal state machine and pure DTOs.
//!
//! No terminal APIs, no I/O. The persistence closure is injected by the
//! binary through `SettingsDialog::apply`. The five tabs and the items
//! each one exposes are locked by `design.md` (D6).
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use thiserror::Error;

/// Pure DTO for the TUI preferences loaded from `[ui]` in `config.toml`.
/// Carries `#[serde(default)]` only — no `deny_unknown_fields` because
/// REQ-11 requires tolerating hand-added keys under `[ui]`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiConfig {
    pub theme: UiTheme,
    pub sound: UiSound,
    pub toasts: UiToasts,
    pub indicators: UiIndicators,
    pub integrations: UiIntegrations,
}

impl UiConfig {
    /// Defaults documented in `proposal.md` and `spec.md` REQ-10.
    pub fn defaults() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiTheme {
    pub name: String,
}
impl Default for UiTheme {
    fn default() -> Self {
        Self {
            name: "one-dark".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiSound {
    pub bell: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiToasts {
    pub level: String,
    pub duration_secs: u32,
}
impl Default for UiToasts {
    fn default() -> Self {
        Self {
            level: "info".into(),
            duration_secs: 3,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiIndicators {
    pub scrollbar: bool,
    pub breadcrumbs: bool,
    pub line_numbers: bool,
}
impl Default for UiIndicators {
    fn default() -> Self {
        Self {
            scrollbar: true,
            breadcrumbs: true,
            line_numbers: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UiIntegrations {
    pub entries: Vec<String>,
}

pub const TAB_NAMES: [&str; 5] = ["theme", "indicators", "sound", "toasts", "integrations"];
pub const TAB_COUNT: usize = TAB_NAMES.len();
pub const TAB_THEME: usize = 0;
pub const TAB_INDICATORS: usize = 1;
pub const TAB_SOUND: usize = 2;
pub const TAB_TOASTS: usize = 3;
pub const TAB_INTEGRATIONS: usize = 4;

/// Persistence closure invoked by the dialog on Enter.
pub type SaveUiConfig = std::sync::Arc<dyn Fn(&UiConfig) -> anyhow::Result<()> + Send + Sync>;

/// Available themes for the `theme` tab (D6 — 13 names).
pub const THEME_NAMES: [&str; 13] = [
    "one-dark",
    "one-light",
    "catppuccin",
    "catppuccin-latte",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "terminal",
    "solarized",
    "solarized-light",
];

/// Cycle values for the toasts-level cycle (D6).
/// `info→0, warn→1, error→2`.
pub const TOAST_LEVELS: [&str; 3] = ["info", "warn", "error"];

/// Cycle values for the toasts-duration cycle (D6).
/// `3→0, 5→1, 10→2, off→3`. Stored as seconds; `off` renders as 0.
pub const TOAST_DURATIONS: [(u32, &str); 4] = [(3, "3s"), (5, "5s"), (10, "10s"), (0, "off")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    Pick,
    Bool,
    Cycle,
    Action,
}

/// Borrowed view of one item in a tab's body.
pub struct SettingsItem<'a> {
    pub label: Cow<'a, str>,
    pub checked: bool,
    pub kind: ItemKind,
    /// When `kind` is `Cycle`, the current cycle index. `None` otherwise.
    pub cycle: Option<usize>,
}

/// Borrowed view of the dialog state, consumed by the renderer.
pub struct SettingsSnapshot<'a> {
    pub title: &'a str,
    pub tabs: [&'a str; 5],
    pub active: usize,
    pub items: Vec<SettingsItem<'a>>,
    pub selected: usize,
    /// Error from the previous `apply` (`None` on first attempt).
    pub error: Option<String>,
}

/// Working-copy of the theme tab: a selected name among `THEME_NAMES`.
///
/// `touched` distinguishes "the user picked a row" from "the dialog started
/// on this row": an untouched tab keeps the persisted name verbatim, so a
/// name outside `THEME_NAMES` is not silently rewritten on save.
#[derive(Debug, Clone)]
pub struct ThemeWorking {
    pub name_index: usize,
    pub touched: bool,
}

/// Working-copy of the indicators tab: three booleans.
#[derive(Debug, Clone)]
pub struct IndicatorsWorking {
    pub scrollbar: bool,
    pub breadcrumbs: bool,
    pub line_numbers: bool,
}

/// Working-copy of the sound tab: a single boolean.
#[derive(Debug, Clone)]
pub struct SoundWorking {
    pub bell: bool,
}

/// Working-copy of the toasts tab: two cycle indices.
#[derive(Debug, Clone)]
pub struct ToastsWorking {
    /// Index into `TOAST_LEVELS`.
    pub level: usize,
    /// Index into `TOAST_DURATIONS`.
    pub duration: usize,
    /// Set when the user changes the level; otherwise the persisted value wins.
    pub level_touched: bool,
    /// Set when the user changes the duration; otherwise the persisted value
    /// wins.
    pub duration_touched: bool,
}

/// Working-copy of the integrations tab. v1 is read-only except for
/// the `(+) Add provider…` action row; `entries` round-trips untouched.
#[derive(Debug, Clone, Default)]
pub struct IntegrationsWorking {
    pub entries: Vec<String>,
}

/// Working copy of all five tabs.
#[derive(Debug, Clone)]
pub struct WorkingCopy {
    pub theme: ThemeWorking,
    pub indicators: IndicatorsWorking,
    pub sound: SoundWorking,
    pub toasts: ToastsWorking,
    pub integrations: IntegrationsWorking,
}

/// Per-tab selection cursor. Persists across tab changes inside one
/// dialog session.
#[derive(Debug, Clone)]
pub struct TabSelection {
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct SettingsDialog {
    pub active_tab: usize,
    pub selections: [TabSelection; 5],
    pub working: WorkingCopy,
    pub last_error: Option<String>,
    /// Values loaded when the dialog opened. Untouched fields project from
    /// here, so an out-of-set value survives a save the user never intended
    /// to change.
    baseline: UiConfig,
    baseline_theme: usize,
    baseline_level: usize,
    baseline_duration: usize,
}

#[derive(Error, Debug)]
pub enum ApplyError {
    #[error("{0}")]
    Persist(String),
}

impl SettingsDialog {
    /// Build from a freshly-loaded `UiConfig`. Selections start at index 0.
    pub fn new(current: &UiConfig) -> Self {
        let name_index = THEME_NAMES
            .iter()
            .position(|name| *name == current.theme.name)
            .unwrap_or(0);
        let level = TOAST_LEVELS
            .iter()
            .position(|level| *level == current.toasts.level)
            .unwrap_or(0);
        let duration = TOAST_DURATIONS
            .iter()
            .position(|(secs, _)| *secs == current.toasts.duration_secs)
            .unwrap_or(0);
        let theme = ThemeWorking {
            name_index,
            touched: false,
        };
        let indicators = IndicatorsWorking {
            scrollbar: current.indicators.scrollbar,
            breadcrumbs: current.indicators.breadcrumbs,
            line_numbers: current.indicators.line_numbers,
        };
        let sound = SoundWorking {
            bell: current.sound.bell,
        };
        let toasts = ToastsWorking {
            level,
            duration,
            level_touched: false,
            duration_touched: false,
        };
        let integrations = IntegrationsWorking {
            entries: current.integrations.entries.clone(),
        };
        let selections = [
            TabSelection { selected: 0 },
            TabSelection { selected: 0 },
            TabSelection { selected: 0 },
            TabSelection { selected: 0 },
            TabSelection { selected: 0 },
        ];
        Self {
            active_tab: 0,
            selections,
            working: WorkingCopy {
                theme,
                indicators,
                sound,
                toasts,
                integrations,
            },
            last_error: None,
            baseline: current.clone(),
            baseline_theme: name_index,
            baseline_level: level,
            baseline_duration: duration,
        }
    }

    pub const fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % TAB_COUNT;
    }

    pub const fn prev_tab(&mut self) {
        self.active_tab = (self.active_tab + TAB_COUNT - 1) % TAB_COUNT;
    }

    const fn item_count(&self) -> usize {
        match self.active_tab {
            TAB_THEME => THEME_NAMES.len(),
            TAB_INDICATORS => 3,
            TAB_SOUND => 1,
            TAB_TOASTS => 2,
            TAB_INTEGRATIONS => 1,
            _ => 0,
        }
    }

    pub const fn select_next(&mut self) {
        let count = self.item_count();
        if count == 0 {
            return;
        }
        let cur = &mut self.selections[self.active_tab].selected;
        *cur = (*cur + 1) % count;
    }

    pub const fn select_prev(&mut self) {
        let count = self.item_count();
        if count == 0 {
            return;
        }
        let cur = &mut self.selections[self.active_tab].selected;
        *cur = (*cur + count - 1) % count;
    }

    /// Bool → flip; Cycle → advance index; Pick → set the working copy
    /// to the named option; Action → no-op.
    pub const fn toggle_selected(&mut self) {
        let selected = self.selections[self.active_tab].selected;
        match self.active_tab {
            TAB_THEME => {
                self.working.theme.name_index = selected;
                self.working.theme.touched = true;
            }
            TAB_INDICATORS => match selected {
                0 => self.working.indicators.scrollbar = !self.working.indicators.scrollbar,
                1 => self.working.indicators.breadcrumbs = !self.working.indicators.breadcrumbs,
                2 => self.working.indicators.line_numbers = !self.working.indicators.line_numbers,
                _ => {}
            },
            TAB_SOUND => {
                if selected == 0 {
                    self.working.sound.bell = !self.working.sound.bell;
                }
            }
            TAB_TOASTS => match selected {
                0 => {
                    let n = TOAST_LEVELS.len();
                    self.working.toasts.level = (self.working.toasts.level + 1) % n;
                    self.working.toasts.level_touched = true;
                }
                1 => {
                    let n = TOAST_DURATIONS.len();
                    self.working.toasts.duration = (self.working.toasts.duration + 1) % n;
                    self.working.toasts.duration_touched = true;
                }
                _ => {}
            },
            TAB_INTEGRATIONS => {}
            _ => {}
        }
    }

    /// Build a `UiConfig` that mirrors the current working copy. The
    /// caller uses this for the persistence projection.
    pub fn projected_ui_config(&self) -> UiConfig {
        let theme_index = self.working.theme.name_index;
        let theme = if self.working.theme.touched || theme_index != self.baseline_theme {
            UiTheme {
                name: THEME_NAMES[theme_index].into(),
            }
        } else {
            self.baseline.theme.clone()
        };
        let level_index = self.working.toasts.level;
        let level = if self.working.toasts.level_touched || level_index != self.baseline_level {
            TOAST_LEVELS[level_index].into()
        } else {
            self.baseline.toasts.level.clone()
        };
        let duration_index = self.working.toasts.duration;
        let duration_secs =
            if self.working.toasts.duration_touched || duration_index != self.baseline_duration {
                TOAST_DURATIONS[duration_index].0
            } else {
                self.baseline.toasts.duration_secs
            };
        UiConfig {
            theme,
            sound: UiSound {
                bell: self.working.sound.bell,
            },
            toasts: UiToasts {
                level,
                duration_secs,
            },
            indicators: UiIndicators {
                scrollbar: self.working.indicators.scrollbar,
                breadcrumbs: self.working.indicators.breadcrumbs,
                line_numbers: self.working.indicators.line_numbers,
            },
            integrations: UiIntegrations {
                entries: self.working.integrations.entries.clone(),
            },
        }
    }

    /// Run the injected persistence closure exactly once with the
    /// projected `UiConfig`. Stores any error message in `last_error`.
    pub fn apply(
        &mut self,
        save: &mut (dyn FnMut(&UiConfig) -> anyhow::Result<()> + Send + Sync),
    ) -> Result<(), ApplyError> {
        let projected = self.projected_ui_config();
        match save(&projected) {
            Ok(()) => {
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.last_error = Some(message.clone());
                Err(ApplyError::Persist(message))
            }
        }
    }

    /// Drop the working copy without calling `save`.
    pub const fn discard(&mut self) {
        // Nothing to free here; the host-owned overlay holds the working copy.
    }

    /// True when the currently selected row is the `(+) Add provider…`
    /// action row on the integrations tab. The driver closes the dialog
    /// and surfaces the notice — no persistence call.
    pub const fn handle_add_provider(&self) -> bool {
        self.active_tab == TAB_INTEGRATIONS && self.selections[TAB_INTEGRATIONS].selected == 0
    }

    /// Borrow view for the renderer.
    pub fn snapshot(&self) -> SettingsSnapshot<'_> {
        let selected = self.selections[self.active_tab].selected;
        let items = self.build_items();
        SettingsSnapshot {
            title: "Settings",
            tabs: TAB_NAMES,
            active: self.active_tab,
            items,
            selected,
            error: self.last_error.clone(),
        }
    }

    fn build_items(&self) -> Vec<SettingsItem<'_>> {
        match self.active_tab {
            TAB_THEME => {
                let working_name = THEME_NAMES[self.working.theme.name_index];
                THEME_NAMES
                    .iter()
                    .map(|name| SettingsItem {
                        label: Cow::Borrowed(*name),
                        checked: *name == working_name,
                        kind: ItemKind::Pick,
                        cycle: None,
                    })
                    .collect()
            }
            TAB_INDICATORS => {
                let ind = &self.working.indicators;
                vec![
                    SettingsItem {
                        label: Cow::Borrowed("scrollbar"),
                        checked: ind.scrollbar,
                        kind: ItemKind::Bool,
                        cycle: None,
                    },
                    SettingsItem {
                        label: Cow::Borrowed("breadcrumbs"),
                        checked: ind.breadcrumbs,
                        kind: ItemKind::Bool,
                        cycle: None,
                    },
                    SettingsItem {
                        label: Cow::Borrowed("line_numbers"),
                        checked: ind.line_numbers,
                        kind: ItemKind::Bool,
                        cycle: None,
                    },
                ]
            }
            TAB_SOUND => {
                let s = &self.working.sound;
                vec![SettingsItem {
                    label: Cow::Borrowed("bell"),
                    checked: s.bell,
                    kind: ItemKind::Bool,
                    cycle: None,
                }]
            }
            TAB_TOASTS => {
                let level_name = TOAST_LEVELS[self.working.toasts.level];
                let (_, dur_label) = TOAST_DURATIONS[self.working.toasts.duration];
                vec![
                    SettingsItem {
                        label: Cow::Owned(format!("level: {level_name}")),
                        checked: true,
                        kind: ItemKind::Cycle,
                        cycle: Some(self.working.toasts.level),
                    },
                    SettingsItem {
                        label: Cow::Owned(format!("duration: {dur_label}")),
                        checked: true,
                        kind: ItemKind::Cycle,
                        cycle: Some(self.working.toasts.duration),
                    },
                ]
            }
            TAB_INTEGRATIONS => vec![SettingsItem {
                label: Cow::Borrowed("(+) Add provider…"),
                checked: false,
                kind: ItemKind::Action,
                cycle: None,
            }],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
