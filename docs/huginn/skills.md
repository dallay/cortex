# Agent Skills Catalog

`huginn skills list` inspects local metadata without configuring a model,
opening a SQLite session, starting MCP, or prompting for interactive input.

```sh
huginn --workspace ./packages/app skills list
huginn --workspace ./packages/app --json skills list
```

## Locations and Precedence

- Global: `$HOME/.agents/skills/<name>/SKILL.md`. Without `HOME`, the global
  scope is not queried.
- Project: `<root>/.agents/skills/<name>/SKILL.md`. Root is the closest ancestor
  of the resolved workspace with a `.git` file or directory marker. Without
  a marker, the resolved workspace is used.

Only one directory level is inspected. Nested collections and additional
per-package scopes are not searched. A valid project entry overrides a global
entry with the exact same name; both remain visible with status `selected`
or `overridden`. Invalid entries do not mask a valid global entry.

Results are ordered by ASCII name, scope (project first), and logical path.
Logical paths preserve the alias used to discover the file; canonical paths
indicate the resolved target.

## Validation and Limits

Each file must be UTF-8 and begin with YAML frontmatter between `---` lines.
`name` and `description` must be strings. The name must match the logical
directory basename exactly: between 1 and 64 lowercase ASCII characters, digits, or
hyphens, without leading, trailing, or consecutive hyphens. The description must
contain non-empty text and at most 1024 Unicode characters.

`compatibility`, if present, must contain between 1 and 500 Unicode characters.
`license` and `allowed-tools` must be strings. `metadata` must be a map of
strings to strings. Additional fields are not interpreted as policy.

Huginn limits each file to 65,536 bytes and each root to 256 direct children,
including non-skill files. A root exceeding this child count is rejected entirely;
the other root is processed independently. These limits are Huginn policy,
not requirements of the Agent Skills specification.

Global symlinks may point outside their root, for instance into a dotfiles
repository. Both the resolved directory and file of a project skill must
remain within the canonical repository. Broken, cyclic, unreadable, or non-regular
targets produce diagnostics.

## JSON Contract

Output is a single JSON object with no additional stdout chatter:

```json
{
  "fingerprint": "<64-character lowercase hex SHA-256>",
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

JSON and human paths replace non-UTF-8 path bytes with U+FFFD without aborting
the command. Native paths remain intact in the typed catalog and fingerprint
calculation; lossy display strings must not be used as identities to activate skills.

Missing roots are not errors. An invalid entry emits a diagnostic and does not
prevent displaying valid entries. Stable diagnostic codes are `root_unreadable`,
`path_resolution`, `root_limit`, `project_escape`, `not_directory`, `not_regular`,
`file_unreadable`, `file_limit`, `source_changed`, `invalid_utf8`,
`frontmatter_missing`, `invalid_yaml`, `metadata_type`, `invalid_name`,
`name_mismatch`, `invalid_description`, and `invalid_compatibility`.
`project_escape` uses severity `security`; others use `error`.

## Freshness and Trust

There is no caching: every query rebuilds the catalog. The port
`SkillCatalogQuery::refresh` re-reads files and recalculates SHA-256, even if an
edit preserves size and mtime. The catalog fingerprint uses ordered entries,
their scope, name, paths, and complete file digest. Overridden entries also
participate; diagnostics do not participate. Paths form part of the identity:
moving a file changes the fingerprint.

The adapter checks metadata and targets before and after reading. If changes
are detected, it rejects the affected entry or root. These checks do not constitute
a sandbox or guarantee isolation against concurrent local processes modifying paths.
Activation revalidates provenance.

All skills are labeled `untrusted`, including global ones. Human output escapes
terminal control and bidirectional control characters; JSON preserves data via
serialization. `description`, `allowed-tools`, and other metadata never grant
permissions. `huginn skills list` only inspects the catalog and does not load
instructions. `AGENTS.md` and `.agents/rules` maintain their previous behavior.

## On-Demand Loading

Native composition advertises global metadata to the model as `untrusted` task
data, with exact identity, scope, and canonical path. The advertisement has a
16 KiB JSON limit; it indicates how many entries were omitted. Host-selected
identities take priority. If their metadata does not fit, the turn fails
explicitly. It does not include bodies or descriptions from project skills without
explicit user selection.

The `load_skill` tool accepts the exact advertised identity, not an arbitrary
path or ambiguous name. It rebuilds the catalog and rechecks scope, paths, and
file digest before returning `SKILL.md`. If provenance changes, disappears, or
becomes invalid, loading fails and requests a catalog refresh. The full JSON
result has a 16 KiB limit; a skill exceeding that limit fails without truncating
instructions. The session context may enforce a lower limit and retains normal
errors and compaction rules.

`StandardLoop::run_with_skills` allows the host to select project identities for
a single turn. This selection does not grant persistent trust. The tool does not
execute scripts, load references automatically, or modify `HostPolicy`.
Subsequent tool calls continue to require standard approvals.

The session records only the versioned catalog fingerprint, without duplicating
bodies in its metadata. Normal tool outputs remain in the original history.
Starting another turn with a differing fingerprint produces a warning showing
both prior and current fingerprints. Previous loads or effects are not repeated,
and bodies are not reinjected outside standard history.

## Explicit Invocation

Use `/skill:<name> [task]` in the TUI, line mode, `run`, or a resumed session:

```sh
huginn --workspace . run '/skill:code-review Review "src/lib.rs" without editing'
huginn --workspace . --json run '/skill:code-review'
huginn resume <uuid> --prompt '/skill:code-review Review new changes'
```

The TUI displays selected names and their descriptions in existing autocompletion.
Tab inserts the command; Enter submits it. It is a host command, not an extension
of the plugin command grammar. The autocompletion catalog is a startup snapshot;
each invocation queries the catalog anew.

The host allows a maximum of 65,536 bytes per invocation. The name follows catalog
rules. It does not accept paths, scripts, or remote plugins. After the name, it
consumes one whitespace character as separator and preserves all remaining task
bytes, including quotes, backslashes, and extra or trailing spaces. It performs no
shell parsing or argument expansion. The task is optional.

Before beginning a turn or querying the model, the host resolves exactly one
`selected` entry and loads its identity via `SkillLoader`. Project precedence over
global remains unchanged. An invalid, unknown, or ambiguous name, an unavailable
load, or cancellation during loading rejects the request without inference,
effects, or artificial assistant turns. JSON errors add no human text to stdout.
Prompt templates cannot indirectly trigger skills; supply the skill command directly.

The task and load result are saved in a user message as a JSON object separated
into `task` and `skill`. The body preserves `untrusted`, identity, scope, and
canonical path; it is never converted into a system message. Neither the body nor
`allowed-tools` grants permissions: `HostPolicy` enforces standard approvals.
Resuming preserves history, but a new invocation revalidates and loads current
provenance. It does not re-execute previous loads or effects.
