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

fn atomic_write(text: &str, path: &Path) -> Result<()> {
    // Follow a symlinked configuration (dotfiles) so the link survives.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot mkdir {}", parent.display()))?;
    let existing = fs::metadata(&target).ok();
    let tmp = target.with_extension("toml.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // An existing file keeps its mode: the configuration can hold
        // credentials. A new file uses a normal creation mode so the kernel
        // applies the process umask.
        let mode = existing
            .as_ref()
            .map(|metadata| metadata.permissions().mode())
            .unwrap_or(0o666);
        options.mode(mode);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("cannot write {}", tmp.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("cannot sync {}", tmp.display()))?;
        if let Some(metadata) = &existing {
            fs::set_permissions(&tmp, metadata.permissions())
                .with_context(|| format!("cannot set permissions on {}", tmp.display()))?;
        }
        fs::rename(&tmp, &target)
            .with_context(|| format!("cannot rename {} -> {}", tmp.display(), target.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        persist_seeded(&ui, &path, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[ui.theme]"));
        assert!(text.contains("name = \"one-dark\""));
        assert!(text.contains("[ui.toasts]"));
        assert!(text.contains("level = \"info\""));
        assert!(text.contains("duration_secs = 3"));
        assert!(text.contains("[ui.indicators]"));
        assert_eq!(crate::config::Config::load(Some(&path)).unwrap().ui, ui);
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
        persist_seeded(&ui, &path, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        // Outside [ui]: untouched.
        assert!(text.contains("# user comment"));
        assert!(text.contains("[provider]"));
        assert!(text.contains("api_key = \"${OPENAI_API_KEY}\""));
        assert!(text.contains("[model]"));
        assert!(text.contains("default = \"gpt-4o\""));
        assert!(text.starts_with(body), "outside-ui bytes must be identical");
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
custom_theme_key = "retain"
[ui.toasts]
duration_secs = 5
custom_toast_key = 17
"#;
        write_file(&path, body);
        let ui = UiConfig::defaults();
        persist_seeded(&ui, &path, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("unknown_user_key = true"));
        // Theme flipped to default yet the unknown key remains.
        assert!(text.contains("name = \"one-dark\""));
        assert!(text.contains("custom_theme_key = \"retain\""));
        assert!(text.contains("custom_toast_key = 17"));
        assert_eq!(crate::config::Config::load(Some(&path)).unwrap().ui, ui);
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
        persist_seeded(&ui, &path, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("name = \"nord\""));
        assert!(text.contains("bell = true"));
        assert!(text.contains("level = \"warn\""));
        assert!(text.contains("duration_secs = 10"));
        assert!(text.contains("scrollbar = false"));
        assert!(text.contains("line_numbers = false"));
        assert_eq!(crate::config::Config::load(Some(&path)).unwrap().ui, ui);
    }

    #[test]
    fn migrates_inline_ui_table_without_losing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(
            &path,
            "ui = { unknown_user_key = true, theme = { name = \"nord\" } }\n",
        );
        persist_seeded(&UiConfig::defaults(), &path, None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("unknown_user_key = true"), "{text}");
        assert!(text.contains("[ui.theme]"), "{text}");
        assert!(text.contains("name = \"one-dark\""), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_existing_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, "provider = \"openai\"\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        persist_seeded(&UiConfig::defaults(), &path, None).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn writes_through_a_symlinked_config() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/huginn.toml");
        write_file(&real, "provider = \"openai\"\n");
        let link = dir.path().join("config.toml");
        symlink(&real, &link).unwrap();

        persist_seeded(&UiConfig::defaults(), &link, None).unwrap();

        let metadata = fs::symlink_metadata(&link).unwrap();
        assert!(metadata.file_type().is_symlink(), "the link must survive");
        let text = fs::read_to_string(&real).unwrap();
        assert!(text.contains("provider = \"openai\""));
        assert!(text.contains("[ui.theme]"));
    }
}
