# Markdown Task Prompt Templates

Huginn discovers user text prompt templates without executing template code.
These templates are distinct from `AGENTS.md` and skill instructions.
They do not alter system instructions or grant permissions for tools.

## Directories and Precedence

- User, macOS and Linux: `~/.huginn/prompts/`.
- User, Windows: `%USERPROFILE%\.huginn\prompts\`.
- Project: `<project-root>/.huginn/prompts/`.

The project root is the nearest ancestor containing `.git`; without a repository,
Huginn uses the workspace. The user root is the system home directory.
These paths do not relocate existing configuration or databases.

Only direct files with lowercase `.md` extensions are discovered. The template
name is the basename without extension: between 1 and 64 ASCII alphanumeric characters,
hyphens, or underscores. Matching is case-sensitive.
A valid project template takes precedence over a user template with the same name.
An invalid file produces a diagnostic and is not selected.
The autocompletion list loads upon opening the TUI; restart it to discover
new names. Invocation re-reads and revalidates the file.

## Invocation

```text
/prompts:review src/lib.rs "thread safety"
```

The TUI provides autocompletion with description and scope. The same syntax works
in `--line-mode` and as an argument to `huginn run`:

```sh
huginn run '/prompts:review src/lib.rs "thread safety"'
```

The `/prompts:` namespace follows the historical Codex pattern. Aliases such as
`/review` are not registered; `/login` and `/skill:` preserve their identity.

## Format and Variables

Example `review.md`:

```markdown
---
description: Code review
argument-hint: file "focus"
---
Review $1 with focus $2; context: $@
```

Frontmatter uses a restricted `key: value` pair format, not generic YAML.
It supports `description`, `argument-hint`, and its alias `argument_hint`. It supports
simple values, outer quotes, blank lines, and full `#` comments.
Malformed or unknown metadata generates errors. Multiline values are not supported
in frontmatter. The body preserves newlines and Unicode.

- `$1` to `$9`: positional arguments; a missing position produces an error.
- `$ARGUMENTS` and `$@`: all arguments joined with a space.
- `$NAME`: named argument, supplied as `NAME=value`. The name contains uppercase
  ASCII letters, digits, and `_`, starting with a letter.
- `$$`: literal `$` sign. For example, `$$HOME` produces `$HOME` without reading the environment.

Single and double quotes group an argument. A backslash escapes the next
character except inside single quotes. Unclosed quotes and incomplete escapes
produce errors. An empty quoted string constitutes an empty argument. Named assignments
also count as positional arguments and participate in the aggregate. Duplicate
assignments use the last value. Named and positional variables can be combined
in a template. `$10` and multi-digit positions produce an error; `$0` remains literal.
There are no default values.

Substitution occurs in a single pass. Inserted values are not re-evaluated as
variables. There are no expressions, shell execution, secret reading, or
environment variable expansion. `$` characters followed by unsupported symbols
remain literal.

## Limits and Security

- Maximum 256 direct entries per directory; exceeding this skips the directory
  with a diagnostic rather than selecting an arbitrary subset.
- Maximum 64 KiB per UTF-8 file.
- Maximum 256 KiB for combined arguments and expanded result.
- Symlinked template directories and symlinked files are not supported.
  Paths are checked at discovery and invocation.

The expanded text enters as a user message in the standard workflow. History
preserves that text, not a reference that changes if the template is edited later.
`HostPolicy` authorizations continue to apply to any tool requested by the model.
Text asking to bypass authorization does not bypass it.
Expansion errors prevent sending the task to the model. In JSON mode,
metadata and catalog diagnostics are not printed to stdout.
