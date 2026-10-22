//! Pure completion state. No callbacks, filesystem work or terminal I/O here.
use crate::state::Editor;
use huginn_plugin_api::v1::{Command, CommandInfo, CommandMetadata};
use std::ops::Range;

#[derive(Clone)]
pub struct CatalogCommand {
    pub name: String,
    pub metadata: CommandMetadata,
    pub command: Option<Command>,
}
impl CatalogCommand {
    pub fn host(name: &str, description: &str) -> Self {
        Self {
            name: name.into(),
            metadata: CommandMetadata {
                description: description.into(),
                ..Default::default()
            },
            command: None,
        }
    }
    fn active(&self) -> bool {
        self.command.as_ref().is_none_or(Command::is_active)
    }
}
impl From<CommandInfo> for CatalogCommand {
    fn from(info: CommandInfo) -> Self {
        Self {
            name: info.name,
            metadata: info.metadata,
            command: Some(info.command),
        }
    }
}
#[derive(Clone)]
pub struct Candidate {
    pub label: String,
    pub description: String,
    insertion: String,
    command: Option<Command>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Command,
    Argument(String),
    File,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    kind: Kind,
    range: Range<usize>,
    query: String,
}
#[derive(Clone, Default)]
pub struct Completion {
    pub items: Vec<Candidate>,
    pub selected: usize,
    context: Option<Context>,
    dismissed: Option<(String, usize)>,
    invalidated: Option<(String, usize)>,
}
pub enum Acceptance {
    Inserted,
    Command(Option<Command>),
}

/// Boundary-aware tokens, including quoted references with spaces. Ranges are
/// byte offsets on UTF-8 boundaries and include the suffix after the cursor.
fn context(editor: &Editor) -> Option<Context> {
    let text = &editor.text;
    let cursor = editor.cursor;
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let start = text.len() - text.trim_start().len();
    let command_end = command_end(text, start);
    if let Some(context) = command_context(text, start, cursor, command_end) {
        return Some(context);
    }

    let (token_start, quoted) = token_start(text, cursor);
    let end = token_end(text, cursor, quoted);
    let token = &text[token_start..cursor];
    if let Some(query) = token.strip_prefix('@') {
        return Some(Context {
            kind: Kind::File,
            range: token_start..end,
            query: unquote(query),
        });
    }
    argument_context(text, start, cursor, command_end, token_start, end, token)
}

fn command_end(text: &str, start: usize) -> usize {
    text[start..]
        .find(char::is_whitespace)
        .map_or(text.len(), |index| start + index)
}

fn command_context(text: &str, start: usize, cursor: usize, end: usize) -> Option<Context> {
    (text[start..].starts_with('/') && cursor > start && cursor <= end).then(|| Context {
        kind: Kind::Command,
        range: start..end,
        query: text[start..cursor].into(),
    })
}

fn token_start(text: &str, cursor: usize) -> (usize, bool) {
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in text[..cursor].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quoted {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            start = index + character.len_utf8();
        }
    }
    (start, quoted)
}

fn token_end(text: &str, cursor: usize, in_quotes: bool) -> usize {
    let mut end = text.len();
    let mut first_whitespace = None;
    let mut in_quotes = in_quotes;
    let mut escaped = false;
    for (index, character) in text[cursor..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && in_quotes {
            escaped = true;
            continue;
        }
        if character == '"' {
            in_quotes = !in_quotes;
        } else if character.is_whitespace() {
            first_whitespace.get_or_insert(cursor + index);
            if !in_quotes {
                end = cursor + index;
                break;
            }
        }
    }
    if in_quotes && end == text.len() {
        first_whitespace.unwrap_or(text.len())
    } else {
        end
    }
}

fn argument_context(
    text: &str,
    start: usize,
    cursor: usize,
    command_end: usize,
    token_start: usize,
    end: usize,
    token: &str,
) -> Option<Context> {
    (text[start..].starts_with('/') && cursor > command_end && token_start > command_end).then(
        || Context {
            kind: Kind::Argument(text[start..command_end].into()),
            range: token_start..end,
            query: unquote(token),
        },
    )
}
fn unquote(text: &str) -> String {
    let Some(text) = text.strip_prefix('"') else {
        return text.into();
    };
    let mut decoded = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(|next| matches!(next, '\\' | '"')) {
            if let Some(escaped) = chars.next() {
                decoded.push(escaped);
            }
        } else if ch != '"' {
            decoded.push(ch);
        }
    }
    decoded
}
fn quote(text: &str) -> String {
    if text
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\\'))
    {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        text.into()
    }
}
/// Every whitespace-separated query token must match in order within the label.
/// Exact and prefix matches outrank subsequences; short gaps win within a class.
fn score_lowercase(label: &str, query: &str) -> Option<(u8, usize)> {
    let label = label.to_lowercase();
    if label == query {
        return Some((0, 0));
    }
    if label.starts_with(query) {
        return Some((1, label.len() - query.len()));
    }
    let mut cost = 0;
    for token in query.split_whitespace() {
        let mut remaining = label.as_str();
        for ch in token.chars() {
            let index = remaining.find(ch)?;
            cost += index;
            remaining = &remaining[index + ch.len_utf8()..];
        }
    }
    Some((2, cost))
}
fn push_ranked(
    ranked: &mut Vec<((u8, usize), Candidate)>,
    label: &str,
    query: &str,
    insertion: impl FnOnce() -> String,
    description: &str,
    command: Option<Command>,
) {
    if let Some(rank) = score_lowercase(label, query) {
        ranked.push((
            rank,
            Candidate {
                label: label.into(),
                insertion: insertion(),
                description: description.into(),
                command,
            },
        ));
    }
}
impl Completion {
    pub fn blocks_submission(&self, editor: &Editor) -> bool {
        self.invalidated
            .as_ref()
            .is_some_and(|(text, cursor)| text == &editor.text && *cursor == editor.cursor)
    }
    fn invalidate(&mut self, editor: &Editor) {
        self.dismiss(editor);
        self.invalidated = Some((editor.text.clone(), editor.cursor));
    }
    pub fn title(&self) -> &'static str {
        match self.context.as_ref().map(|ctx| &ctx.kind) {
            Some(Kind::Command) => "Commands",
            Some(Kind::Argument(_)) => "Arguments",
            _ => "Files",
        }
    }
    pub fn dismiss(&mut self, editor: &Editor) {
        self.dismissed = Some((editor.text.clone(), editor.cursor));
        self.items.clear();
    }
    pub fn navigate(&mut self, down: bool) {
        if down {
            self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }
    pub fn refresh(&mut self, editor: &Editor, commands: &[CatalogCommand], files: &[String]) {
        if self
            .dismissed
            .as_ref()
            .is_some_and(|(text, cursor)| text == &editor.text && *cursor == editor.cursor)
        {
            return;
        }
        self.dismissed = None;
        self.invalidated = None;
        let next = context(editor);
        // Never silently replace a selected registration with a new generation.
        if self.context == next
            && self.items.iter().any(|item| {
                item.command
                    .as_ref()
                    .is_some_and(|command| !command.is_active())
            })
        {
            self.invalidate(editor);
            return;
        }
        let previous = if self.context == next {
            self.items.get(self.selected).map(|item| item.label.clone())
        } else {
            None
        };
        self.context = next;
        self.items.clear();
        let Some(ctx) = &self.context else {
            return;
        };
        let query = ctx.query.to_lowercase();
        let mut ranked = vec![];
        match &ctx.kind {
            Kind::Command => {
                for command in commands.iter().filter(|command| command.active()) {
                    let description = if command.metadata.argument_hint.is_empty() {
                        command.metadata.description.clone()
                    } else {
                        format!(
                            "{} — {}",
                            command.metadata.argument_hint, command.metadata.description
                        )
                    };
                    push_ranked(
                        &mut ranked,
                        &command.name,
                        &query,
                        || command.name.clone(),
                        &description,
                        command.command.clone(),
                    );
                }
            }
            Kind::File => {
                for path in files {
                    push_ranked(
                        &mut ranked,
                        path,
                        &query,
                        || format!("@{}", quote(path)),
                        "Workspace file",
                        None,
                    );
                }
            }
            Kind::Argument(name) => {
                if let Some(command) = commands
                    .iter()
                    .find(|command| &command.name == name && command.active())
                {
                    for value in &command.metadata.arguments {
                        push_ranked(
                            &mut ranked,
                            value,
                            &query,
                            || quote(value),
                            &command.metadata.argument_hint,
                            command.command.clone(),
                        );
                    }
                    if command.metadata.workspace_paths {
                        for path in files {
                            push_ranked(
                                &mut ranked,
                                path,
                                &query,
                                || quote(path),
                                "Workspace path",
                                command.command.clone(),
                            );
                        }
                    }
                }
            }
        }
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        self.items = ranked.into_iter().take(100).map(|(_, item)| item).collect();
        self.selected = previous
            .and_then(|label| self.items.iter().position(|item| item.label == label))
            .unwrap_or(0);
    }
    pub fn accept(&mut self, editor: &mut Editor, enter: bool) -> Option<Acceptance> {
        let ctx = self.context.clone()?;
        if context(editor).as_ref() != Some(&ctx) {
            return None;
        }
        let item = self.items.get(self.selected)?.clone();
        if item
            .command
            .as_ref()
            .is_some_and(|command| !command.is_active())
        {
            self.invalidate(editor);
            return Some(Acceptance::Inserted);
        }
        editor
            .text
            .replace_range(ctx.range.clone(), &item.insertion);
        editor.cursor = ctx.range.start + item.insertion.len();
        let result = if enter && ctx.kind == Kind::Command {
            Acceptance::Command(item.command)
        } else {
            Acceptance::Inserted
        };
        self.dismiss(editor);
        Some(result)
    }
}
