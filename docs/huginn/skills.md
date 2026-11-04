# Agent Skills catalog

`huginn skills list` inspects local metadata without configuring a model,
opening an SQLite session, starting MCP, or requesting interactive input.

```sh
huginn --workspace ./packages/app skills list
huginn --workspace ./packages/app --json skills list
```

## Locations and precedence

- Global: `$HOME/.agents/skills/<name>/SKILL.md`. Without `HOME`, the
  global scope is not consulted.
- Project: `<root>/.agents/skills/<name>/SKILL.md`. The root is the
  nearest ancestor of the resolved workspace containing a `.git` file or
  directory marker. Without a marker, the resolved workspace is used.

Only one level of directories is inspected. Nested collections and
additional per-package scopes are not searched. A valid project entry overrides a global
entry with the exact same name; both remain visible with status `selected`
or `overridden`. Invalid entries do not hide a valid global entry.

Results are sorted by ASCII name, scope (project first), and logical
path. Logical paths preserve the alias used to discover the file;
canonical paths indicate the resolved destination.

## Validation and limits

Each file must be UTF-8 and begin with YAML frontmatter delimited by `---` lines.
`name` and `description` must be strings. The name must exactly match
the logical directory name: 1 to 64 lowercase ASCII characters, digits, or
hyphens, with no leading, trailing, or consecutive hyphens. The description must
contain non-empty text with at most 1024 Unicode characters.

`compatibility`, if present, must contain between 1 and 500 Unicode characters.
`license` and `allowed-tools` must be strings. `metadata` must be a map of
strings to strings. Additional fields are not interpreted as policies.

Huginn limits each file to 65,536 bytes and each root to 256 direct children,
including files that are not skill directories. A root with more children
is rejected entirely; the other root is processed independently. These
limits are Huginn policy, not requirements of the Agent Skills specification.

Global links can point outside their root, for example to a dotfiles
repository. Both the resolved directory and file of a project skill
must remain inside the canonical repository. Broken, cyclic, unreadable,
or non-regular targets produce diagnostics.

## JSON contract

The output is a single object, with no additional messages on stdout:

```json
{
  "fingerprint": "<64-character hexadecimal SHA-256>",
  "skills": [
    {
      "name": "code-review",
      "description": "Review local changes",
      "source_scope": "project",
      "logical_path": "/repo/.agents/skills/code-review/SKILL.md",
      "canonical_path": "/repo/.agents/skills/code-review/SKILL.md",
      "content_fingerprint": "<SHA-256 of the complete file>",
      "trust": "untrusted",
      "selected": true,
      "status": "selected"
    }
  ],
  "diagnostics": [
    {
      "code": "name_mismatch",
      "scope": "global",
      "path": "/home/user/.agents/skills/alias/SKILL.md",
      "message": "name must exactly match the logical directory basename",
      "severity": "error"
    }
  ]
}
```

JSON and human-readable paths convert non-UTF-8 path bytes via substitution
with U+FFFD, without aborting the command. Native paths remain intact in the
typed catalog and in fingerprint computation; the displayed lossy string must not
be used as identity to activate a skill.

Missing roots are not errors. An invalid entry produces a diagnostic
and does not prevent displaying valid entries. The stable codes are `root_unreadable`,
`path_resolution`, `root_limit`, `project_escape`, `not_directory`, `not_regular`,
`file_unreadable`, `file_limit`, `source_changed`, `invalid_utf8`,
`frontmatter_missing`, `invalid_yaml`, `metadata_type`, `invalid_name`,
`name_mismatch`, `invalid_description`, and `invalid_compatibility`.
`project_escape` uses severity `security`; the others use `error`.

## Freshness and trust

There is no cache: each query rebuilds the catalog. The
`SkillCatalogQuery::refresh` port re-reads and recomputes SHA-256, even if an
edit preserves the size and modification date. The catalog fingerprint
uses the sorted entries, their scope, name, paths, and complete-file digest.
Overridden entries also participate; diagnostics do not participate.
Paths are part of the identity: moving a file changes the fingerprint.

The adapter checks metadata and targets before and after reading. If it detects
changes, it rejects the entry or the affected root. These checks are not a
sandbox and do not guarantee isolation against a local process modifying paths
concurrently. Activation re-validates the origin.

Every skill is labeled `untrusted`, including the global one. Human-readable output escapes
terminal controls and bidirectional controls; JSON preserves the data
through serialization. `description`, `allowed-tools`, and other metadata never
grant permissions. `huginn skills list` only inspects the catalog and does not load
instructions. `AGENTS.md` and `.agents/rules` retain their previous behavior.

## On-demand loading

Native composition announces global metadata to the model as `untrusted` task
data, with exact identity, scope, and canonical path. The announcement has a
16 KiB limit on JSON data; the announcement indicates how many entries are omitted.
Identities selected by the host take priority. If their metadata
does not fit, the turn fails explicitly. It does not include
bodies or project descriptions without explicit user selection.

The `load_skill` tool accepts the exact announced identity, not an
arbitrary path or an ambiguous name. It rebuilds the catalog and re-checks
scope, paths, and file fingerprint before delivering `SKILL.md`. If the source
changes, disappears, or becomes invalid, loading fails and requests an update of the
catalog. The complete JSON result has a 16 KiB limit; a skill that
exceeds that limit fails without truncating instructions. The session context may
impose a lower limit and retains its normal errors and compaction rules.

`StandardLoop::run_with_skills` allows the host to select project
identities for a single turn. This selection does not grant persistent trust.
The tool does not run scripts, automatically load references, or modify `HostPolicy`.
Subsequent tool calls still require their approvals.

The session saves only the versioned catalog fingerprint, without duplicating
bodies in its metadata. Normal tool results remain in
the original history. When starting another turn, a different fingerprint produces a
warning with the previous and current fingerprints. Previous loads or effects are not
repeated and bodies are not reinjected outside the normal history.

## Explicit invocation

Use `/skill:<name> [task]` in the TUI, line mode, `run`, or a resumed session:

```sh
huginn --workspace . run '/skill:code-review Review "src/lib.rs" without editing'
huginn --workspace . --json run '/skill:code-review'
huginn resume <uuid> --prompt '/skill:code-review Review the new changes'
```

The TUI shows selected names and their descriptions in the existing
autocomplete. Tab inserts the command; Enter sends it. It is a host command, not an
extension of the plugin command grammar. The autocomplete catalog
is a startup snapshot; each invocation queries the catalog again.

The host supports a maximum of 65,536 bytes per invocation. The name follows the
catalog rules. It does not support paths, scripts, or remote plugins. After the name, it consumes
one whitespace character as a separator and preserves all remaining bytes
of the task, including quotes, backslashes, and extra or trailing spaces.
It does not perform shell parsing or argument expansion. The task is optional.

Before starting the turn or querying the model, the host resolves exactly one
`selected` entry and loads its identity via `SkillLoader`. The precedence of
project over global does not change. An invalid, unknown, or ambiguous name, an
unavailable load, or a cancellation during loading rejects the request without
inference, effects, or an artificial assistant turn. JSON errors do not add
human-readable text to stdout. Prompt templates cannot activate skills
indirectly; enter the skill command directly.

The task and the load result are saved in a user message as a
JSON object separated into `task` and `skill`. The body retains `untrusted`, identity, scope,
and canonical path; it is never turned into a system message. Neither the body nor
`allowed-tools` grant permissions: `HostPolicy` retains the normal approvals.
Resuming preserves the history, but a new invocation re-validates and loads
the current source. It does not replay previous loads or effects.
