# Markdown task templates

Huginn discovers user-provided text templates without executing template code.
These templates are distinct from `AGENTS.md` and from skill instructions.
They do not change system instructions or grant tool permissions.

## Directories and precedence

- User, macOS and Linux: `~/.huginn/prompts/`.
- User, Windows: `%USERPROFILE%\.huginn\prompts\`.
- Project: `<project-root>/.huginn/prompts/`.

The project root is the nearest ancestor containing `.git`; without a repository,
Huginn uses the workspace. The user root is the system's personal directory.
These paths do not move the current configuration or the database.

Only direct files with a lowercase `.md` extension are discovered. The name
is the basename without extension: 1 to 64 ASCII alphanumeric characters,
hyphens, or underscores. Comparison is case-sensitive.
A valid project template takes precedence over a user template with
the same name. An invalid file produces a diagnostic and is not selected.
The autocomplete list loads when the TUI opens; restart it to discover
new names. Invocation re-reads and validates the file.

## Invocation

```text
/prompts:review src/lib.rs "thread safety"
```

The TUI offers autocomplete with description and scope. The same text works
in `--line-mode` and as an argument to `huginn run`:

```sh
huginn run '/prompts:review src/lib.rs "thread safety"'
```

The `/prompts:` namespace follows the historical Codex pattern. No
aliases such as `/review` are registered; `/login` and `/skill:` retain their identity.

## Format and variables

Example `review.md`:

```markdown
---
description: Review code
argument-hint: file "focus"
---
Review $1 with focus $2; context: $@
```

The frontmatter is a limited `key: value` pair format, not general YAML.
It supports `description`, `argument-hint`, and its alias `argument_hint`. It supports
simple values and outer quotes, blank lines, and full-line comments with `#`.
Malformed or unknown metadata produces errors. Multiline
values are not supported in the frontmatter. The body preserves line breaks and Unicode.

- `$1` through `$9`: positional arguments; a missing position produces an error.
- `$ARGUMENTS` and `$@`: all arguments joined with a space.
- `$NAME`: named argument, supplied as `NAME=value`. The name contains
  uppercase ASCII letters, digits, and `_`, and starts with a letter.
- `$$`: literal `$` sign. For example, `$$HOME` produces `$HOME` without reading the environment.

Single and double quotes group an argument. A backslash escapes the
following character except within single quotes. Incomplete quotes or escapes
produce errors. An empty quoted string is an empty
argument. Named assignments also count as positional arguments
and form part of the aggregate. A duplicate assignment uses the last value.
Named and positional variables can be combined in a template.
`$10` and other multi-digit positions produce an error; `$0` remains literal.
There are no default values.

Substitution happens exactly once. Inserted values are not reinterpreted
as variables. There are no expressions, shell execution, secret reading, or
environment variable expansion. `$` characters followed by unsupported symbols
remain literal.

## Limits and security

- Maximum 256 direct entries per directory; if exceeded, that
  directory is skipped with a diagnostic instead of selecting an arbitrary subset.
- Maximum 64 KiB per UTF-8 file.
- Maximum 256 KiB of arguments and expanded result.
- Template directories relocated via symlinks, and symlink files, are not
  supported. Paths are checked both at discovery and at invocation.

The expanded text enters the normal flow as a user message. The history
retains that text, not a reference that changes if the template is edited later.
`HostPolicy` authorizations still apply to any tool
the model requests. Text that asks to skip an authorization does not skip it.
Expansion errors prevent sending that task to the model. In JSON mode, no
metadata or catalog diagnostics are printed to stdout.
