use huginn_plugin_api::v1::CommandMetadata;
use huginn_presentation::{
    completion::{Acceptance, CatalogCommand, Completion},
    state::Editor,
};

fn commands() -> Vec<CatalogCommand> {
    vec![
        CatalogCommand::host("/compact", "Summarize"),
        CatalogCommand::host("/help", "Help"),
    ]
}
#[test]
fn fuzzy_command_selection_preserves_suffix_and_tab_never_executes() {
    let mut editor = Editor {
        text: "/cpt resto".into(),
        cursor: 4,
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &commands(), &[]);
    assert_eq!(completion.items[0].label, "/compact");
    assert!(matches!(
        completion.accept(&mut editor, false),
        Some(Acceptance::Inserted)
    ));
    assert_eq!(editor.text, "/compact resto");
    assert_eq!(editor.cursor, 8);
}
#[test]
fn mentions_use_utf8_ranges_not_email_and_enter_only_inserts() {
    let mut editor = Editor {
        text: "你 mira @src/ñe viejo".into(),
        cursor: "你 mira @src/ñe".len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &[], &["src/ñame.rs".into()]);
    assert!(matches!(
        completion.accept(&mut editor, true),
        Some(Acceptance::Inserted)
    ));
    assert_eq!(editor.text, "你 mira @src/ñame.rs viejo");
    completion.refresh(
        &Editor {
            text: "a@src".into(),
            cursor: 5,
        },
        &[],
        &["src/file".into()],
    );
    assert!(completion.items.is_empty());
}
#[test]
fn dismissal_stays_closed_until_editor_changes_and_arguments_are_declared() {
    let mut editor = Editor {
        text: "/".into(),
        cursor: 1,
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &commands(), &[]);
    completion.dismiss(&editor);
    completion.refresh(&editor, &commands(), &[]);
    assert!(completion.items.is_empty());
    editor.insert("h");
    completion.refresh(&editor, &commands(), &[]);
    assert_eq!(completion.items[0].label, "/help");
    let mut command = CatalogCommand::host("/format", "Format");
    command.metadata = CommandMetadata {
        arguments: vec!["json".into(), "text".into()],
        ..Default::default()
    };
    editor = Editor {
        text: "/format js".into(),
        cursor: 10,
    };
    completion.refresh(&editor, &[command], &[]);
    assert_eq!(completion.items[0].label, "json");
    assert!(matches!(
        completion.accept(&mut editor, true),
        Some(Acceptance::Inserted)
    ));
    assert_eq!(editor.text, "/format json");
    completion.refresh(&editor, &[], &[]);
    assert!(completion.items.is_empty());
}
#[test]
fn quoted_file_completion_and_multitoken_fuzzy_preserve_reference() {
    let mut editor = Editor {
        text: "mira @\"docs/gu de\" después".into(),
        cursor: "mira @\"docs/gu de".len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &[], &["docs/guía de uso.md".into()]);
    assert_eq!(completion.items.len(), 1);
    completion.accept(&mut editor, true);
    assert_eq!(editor.text, "mira @\"docs/guía de uso.md\" después");
}

#[test]
fn escaped_quote_at_cursor_matches_literal_quote_in_filename() {
    let text = "mira @\"a\\\"";
    let mut editor = Editor {
        text: text.into(),
        cursor: text.len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &[], &["a\"b.txt".into()]);
    assert_eq!(completion.items.len(), 1);
    completion.accept(&mut editor, true);
    assert_eq!(editor.text, "mira @\"a\\\"b.txt\"");
}
