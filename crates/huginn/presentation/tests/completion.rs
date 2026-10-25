use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    Result,
};
use huginn_plugin_api::v1::{
    contributions_id, CommandMetadata, CommandResult, Contributions, UiService, View,
};
use huginn_presentation::{
    completion::{Acceptance, CatalogCommand, Completion},
    state::Editor,
};
use std::sync::Arc;

struct Host(Arc<Contributions>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}

struct PluginCommand;
#[async_trait]
impl Plugin for PluginCommand {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "completion-plugin".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        ui.register_command(
            ctx,
            "completion-plugin.command".into(),
            "/plugin-command".into(),
            CommandMetadata {
                arguments: vec!["short".into()],
                workspace_paths: true,
                ..Default::default()
            },
            Arc::new(|args| Ok(CommandResult::Notice(View::text(args)))),
        )?;
        Ok(())
    }
}

async fn active_plugin_command() -> (Supervisor, Arc<Contributions>, Vec<CatalogCommand>) {
    let contributions = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host(contributions.clone())))
        .unwrap();
    supervisor.register(Box::new(PluginCommand)).unwrap();
    supervisor.start().await.unwrap();
    (
        supervisor,
        contributions.clone(),
        contributions
            .commands()
            .into_iter()
            .map(Into::into)
            .collect(),
    )
}

fn active_command_snapshot_from_supervisor(contributions: &Contributions) -> Vec<CatalogCommand> {
    contributions
        .commands()
        .into_iter()
        .map(Into::into)
        .collect()
}

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
fn completing_unclosed_quoted_mention_preserves_the_draft_suffix() {
    let text = "read @\"docs/gu and then fix it";
    let mut editor = Editor {
        text: text.into(),
        cursor: "read @\"docs/gu".len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &[], &["docs/guía de uso.md".into()]);

    assert_eq!(completion.items.len(), 1);
    completion.accept(&mut editor, false);
    assert_eq!(editor.text, "read @\"docs/guía de uso.md\" and then fix it");
}

#[test]
fn completing_unclosed_quoted_argument_preserves_the_draft_suffix() {
    let text = "/format \"js rest of draft";
    let mut editor = Editor {
        text: text.into(),
        cursor: "/format \"js".len(),
    };
    let mut completion = Completion::default();
    let mut format = CatalogCommand::host("/format", "Format");
    format.metadata = CommandMetadata {
        arguments: vec!["json".into(), "text".into()],
        ..Default::default()
    };
    completion.refresh(&editor, &[format], &[]);

    assert_eq!(completion.items.len(), 1);
    completion.accept(&mut editor, false);
    assert_eq!(editor.text, "/format json rest of draft");
}

#[tokio::test]
async fn revoked_plugin_selection_blocks_submit_until_editor_changes() {
    let (mut supervisor, _contributions, commands) = active_plugin_command().await;
    let mut editor = Editor {
        text: "/plugin-command".into(),
        cursor: "/plugin-command".len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &commands, &[]);
    assert_eq!(completion.items.len(), 1);

    let mut argument_editor = Editor {
        text: "/plugin-command s".into(),
        cursor: "/plugin-command s".len(),
    };
    let mut argument_completion = Completion::default();
    argument_completion.refresh(&argument_editor, &commands, &["src/file.rs".into()]);
    assert!(argument_completion
        .items
        .iter()
        .any(|candidate| candidate.label == "src/file.rs"));
    argument_completion.selected = argument_completion
        .items
        .iter()
        .position(|candidate| candidate.label == "src/file.rs")
        .unwrap();
    argument_completion.accept(&mut argument_editor, false);
    assert_eq!(argument_editor.text, "/plugin-command src/file.rs");

    supervisor.remove("completion-plugin").await.unwrap();
    completion.refresh(&editor, &commands, &[]);
    assert!(completion.items.is_empty());
    assert!(completion.blocks_submission(&editor));
    assert!(completion.accept(&mut editor, true).is_none());

    editor.text.push('x');
    editor.cursor += 1;
    completion.refresh(&editor, &[], &[]);
    assert!(!completion.blocks_submission(&editor));
}

#[tokio::test]
async fn replacement_registration_is_not_selected_for_revoked_generation() {
    let (mut supervisor, contributions, old_commands) = active_plugin_command().await;
    let editor = Editor {
        text: "/plugin-command".into(),
        cursor: "/plugin-command".len(),
    };
    let mut completion = Completion::default();
    completion.refresh(&editor, &old_commands, &[]);
    supervisor.remove("completion-plugin").await.unwrap();
    completion.refresh(&editor, &old_commands, &[]);
    assert!(completion.blocks_submission(&editor));

    supervisor.register(Box::new(PluginCommand)).unwrap();
    supervisor.start().await.unwrap();
    let fresh = active_command_snapshot_from_supervisor(&contributions);
    completion.refresh(&editor, &fresh, &[]);
    assert!(completion.blocks_submission(&editor));
    assert!(completion.items.is_empty());
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
