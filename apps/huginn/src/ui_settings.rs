//! Patches the user's `config.toml` to persist the `[ui]` section.
//!
//! Uses `toml_edit::DocumentMut` so the helper preserves comments,
//! ordering, and unknown tables outside `[ui]` (REQ-09). The writer
//! is path-agnostic: the binary resolves the path in `main.rs` and
//! passes it into the closure.
use anyhow::{anyhow, Context, Result};
use huginn_presentation::settings::UiConfig;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

/// Persist `[ui]` into `target`, seeding the document from `seed` when the
/// process read its configuration from a legacy file.
///
/// The legacy file is never modified: its bytes become the base of the new
/// file, so the user keeps provider, model and database settings while the
/// loader stops seeing the deprecated path.
pub fn persist_seeded(ui: &UiConfig, target: &Path, seed: Option<&Path>) -> Result<()> {
    let mut doc = if target.exists() {
        load_or_init(target)?
    } else if let Some(seed) = seed.filter(|seed| seed.exists()) {
        load_or_init(seed)?
    } else {
        DocumentMut::new()
    };
    write_ui(&mut doc, ui);
    atomic_write(&doc.to_string(), target)
}

fn load_or_init(path: &Path) -> Result<DocumentMut> {
    if path.exists() {
        let text =
            // Host config/save path or migration seed; no model/UI filepath.
            // Evidence: docs/security/semgrep-path-exceptions.md#huginn-ui-settings
            // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
            fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        text.parse::<DocumentMut>()
            .with_context(|| format!("invalid TOML in {}", path.display()))
    } else {
        Ok(DocumentMut::new())
    }
}

fn write_ui(doc: &mut DocumentMut, ui: &UiConfig) {
    let ui_table = ensure_table(doc, "ui");
    let theme = ensure_sub_table(ui_table, "theme");
    theme["name"] = value(ui.theme.name.clone());
    let sound = ensure_sub_table(ui_table, "sound");
    sound["bell"] = value(ui.sound.bell);
    let toasts = ensure_sub_table(ui_table, "toasts");
    toasts["level"] = value(ui.toasts.level.clone());
    toasts["duration_secs"] = value(ui.toasts.duration_secs as i64);
    let indicators = ensure_sub_table(ui_table, "indicators");
    indicators["scrollbar"] = value(ui.indicators.scrollbar);
    indicators["breadcrumbs"] = value(ui.indicators.breadcrumbs);
    indicators["line_numbers"] = value(ui.indicators.line_numbers);
    let integrations = ensure_sub_table(ui_table, "integrations");
    let entries: Array = ui
        .integrations
        .entries
        .iter()
        .map(|s| Value::from(s.clone()))
        .collect();
    integrations["entries"] = Item::Value(entries.into());
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.get(key).is_some_and(|item| item.is_table()) {
        let mut table = Table::new();
        // A hand-written inline table (`ui = { ... }`) holds the same keys;
        // migrate them instead of discarding the user's values.
        if let Some(Item::Value(Value::InlineTable(inline))) = doc.get(key) {
            for (name, item) in inline.iter() {
                table.insert(name, Item::Value(item.clone()));
            }
        }
        doc[key] = Item::Table(table);
    }
    doc[key].as_table_mut().expect("just inserted")
}

fn ensure_sub_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    if !parent.get(key).is_some_and(|item| item.is_table()) {
        let mut table = Table::new();
        if let Some(Item::Value(Value::InlineTable(inline))) = parent.get(key) {
            for (name, item) in inline.iter() {
                table.insert(name, Item::Value(item.clone()));
            }
        }
        parent[key] = Item::Table(table);
    }
    parent[key].as_table_mut().expect("just inserted")
}

pub fn atomic_write(text: &str, path: &Path) -> Result<()> {
    // Follow a symlinked configuration (dotfiles) so the link survives.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = target
        .parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .ok_or_else(|| anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot mkdir {}", parent.display()))?;
    let existing = fs::metadata(&target).ok();
    // NamedTempFile creates an exclusive, private (0600 on Unix) file in the
    // destination directory. RAII attempts cleanup on early errors; a failed
    // persist explicitly closes the file and reports any cleanup failure.
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("cannot create temporary file in {}", parent.display()))?;
    file.write_all(text.as_bytes())?;
    if let Some(metadata) = &existing {
        file.as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("cannot set permissions on {}", file.path().display()))?;
    }
    file.as_file().sync_all()?;
    file.persist(&target)
        .map_err(|failure| {
            let tempfile::PersistError { error, file } = failure;
            // Do not retain the temporary file in the returned error.
            match file.close() {
                Ok(()) => anyhow::Error::from(error),
                Err(cleanup_error) => anyhow::Error::from(error).context(format!(
                    "cannot remove temporary configuration file: {cleanup_error}"
                )),
            }
        })
        .with_context(|| format!("cannot persist configuration to {}", target.display()))?;
    Ok(())
}
