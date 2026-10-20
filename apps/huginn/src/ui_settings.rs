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

    #[cfg(unix)]
    #[test]
    fn persist_leaves_preexisting_temp_symlink_and_victim_intact() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let victim = dir.path().join("victim.txt");
        let tmp = target.with_extension("toml.tmp");
        fs::write(&victim, "do not modify").unwrap();
        symlink(&victim, &tmp).unwrap();
        let ui = UiConfig::defaults();

        persist_seeded(&ui, &target, None).unwrap();

        assert_eq!(fs::read_to_string(&victim).unwrap(), "do not modify");
        assert_eq!(fs::read_link(&tmp).unwrap(), victim);
        assert_eq!(crate::config::Config::load(Some(&target)).unwrap().ui, ui);
    }

    #[test]
    fn persist_leaves_preexisting_regular_temp_intact() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let tmp = target.with_extension("toml.tmp");
        fs::write(&tmp, "do not modify").unwrap();
        let ui = UiConfig::defaults();

        persist_seeded(&ui, &target, None).unwrap();

        assert_eq!(fs::read_to_string(&tmp).unwrap(), "do not modify");
        assert_eq!(crate::config::Config::load(Some(&target)).unwrap().ui, ui);
    }

    #[cfg(unix)]
    #[test]
    fn new_config_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        persist_seeded(&UiConfig::defaults(), &target, None).unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & 0o177, 0, "only the owner may read or write");
    }

    #[test]
    fn failed_replace_preserves_destination_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        fs::create_dir(&target).unwrap();
        let sentinel = target.join("sentinel");
        fs::write(&sentinel, "do not modify").unwrap();

        let error = atomic_write("new content", &target).unwrap_err();

        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "do not modify");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(error.to_string().contains("cannot persist configuration"));
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
        for expected in [0o400, 0o600, 0o640] {
            fs::set_permissions(&path, fs::Permissions::from_mode(expected)).unwrap();

            persist_seeded(&UiConfig::defaults(), &path, None).unwrap();

            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, expected);
        }
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
