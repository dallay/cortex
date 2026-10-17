//! Patches the user's `config.toml` to persist the `[ui]` section.
//!
//! Uses `toml_edit::DocumentMut` so the helper preserves comments,
//! ordering, and unknown tables outside `[ui]` (REQ-09). The writer
//! is path-agnostic: the binary resolves the path in `main.rs` and
//! passes it into the closure.
use anyhow::{anyhow, Context, Result};
use huginn_presentation::settings::UiConfig;
use std::fs;
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table, Value};

/// Upsert the `[ui]` section into `path`, preserving everything else.
///
/// Errors wrap with file context for the closure caller.
pub fn persist(ui: &UiConfig, path: &Path) -> Result<()> {
    let mut doc = load_or_init(path)?;
    write_ui(&mut doc, ui);
    atomic_write(&doc.to_string(), path)
}

fn load_or_init(path: &Path) -> Result<DocumentMut> {
    if path.exists() {
        let text =
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
    let need = match doc.get(key) {
        None => true,
        Some(item) => !item.is_table(),
    };
    if need {
        doc[key] = Item::Table(Table::new());
    }
    doc[key].as_table_mut().expect("just inserted")
}

fn ensure_sub_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    let need = match parent.get(key) {
        None => true,
        Some(item) => !item.is_table(),
    };
    if need {
        parent[key] = Item::Table(Table::new());
    }
    parent[key].as_table_mut().expect("just inserted")
}

fn atomic_write(text: &str, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot mkdir {}", parent.display()))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("cannot rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn writes_to_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let ui = UiConfig::defaults();
        persist(&ui, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[ui.theme]"));
        assert!(text.contains("name = \"one-dark\""));
        assert!(text.contains("[ui.toasts]"));
        assert!(text.contains("level = \"info\""));
        assert!(text.contains("duration_secs = 3"));
        assert!(text.contains("[ui.indicators]"));
    }

    #[test]
    fn preserves_unrelated_tables_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let body = r#"# user comment
[provider]
name = "openai"
api_key = "${OPENAI_API_KEY}"

[model]
default = "gpt-4o"
"#;
        write_file(&path, body);
        let mut ui = UiConfig::defaults();
        ui.theme.name = "catppuccin".into();
        persist(&ui, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        // Outside [ui]: untouched.
        assert!(text.contains("# user comment"));
        assert!(text.contains("[provider]"));
        assert!(text.contains("api_key = \"${OPENAI_API_KEY}\""));
        assert!(text.contains("[model]"));
        assert!(text.contains("default = \"gpt-4o\""));
        // Inside [ui]: patched correctly.
        assert!(text.contains("[ui.theme]"));
        assert!(text.contains("name = \"catppuccin\""));
    }

    #[test]
    fn preserves_inner_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let body = r#"[ui]
unknown_user_key = true
[ui.theme]
name = "dracula"
"#;
        write_file(&path, body);
        let ui = UiConfig::defaults();
        persist(&ui, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("unknown_user_key = true"));
        // Theme flipped to default yet the unknown key remains.
        assert!(text.contains("name = \"one-dark\""));
    }

    #[test]
    fn round_trip_full_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut ui = UiConfig::defaults();
        ui.theme.name = "nord".into();
        ui.sound.bell = true;
        ui.toasts.level = "warn".into();
        ui.toasts.duration_secs = 10;
        ui.indicators.scrollbar = false;
        ui.indicators.line_numbers = false;
        persist(&ui, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("name = \"nord\""));
        assert!(text.contains("bell = true"));
        assert!(text.contains("level = \"warn\""));
        assert!(text.contains("duration_secs = 10"));
        assert!(text.contains("scrollbar = false"));
        assert!(text.contains("line_numbers = false"));
    }
}
