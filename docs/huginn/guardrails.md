# Restrictive Path and Shell Library

## Status

The host can register `GuardsPlugin` and compose guarded native adapters.
The CLI guards run/chat/resume with this composition, without disable flags.
It loads a snapshot from `session.workspace` after creating or restoring the session.

## Explicit Usage

```rust
use huginn_guards::Guardrails;
use std::path::Path;

let policy = Guardrails::from_toml("block_paths=['private']")?;
policy.check_path(Path::new("/workspace"), Path::new("private/file"))?;
```

The host decides whether to invoke the library. An allowed result does not authorize
an effect or substitute `HostPolicy`, approvals, or workspace validation.

## Configuration from Text

`from_toml` accepts TOML text up to 64 KiB with `block_paths` and `block_commands`.
Supports up to 128 relative paths of up to 1024 bytes, without traversal or control characters.
Supports up to 128 additional executables, each non-empty, up to 128 bytes,
with ASCII alphanumeric, dot, hyphen, or underscore characters. Compares names
case-insensitively (ASCII) and merges them with the built-in catalog.
Rejects unknown fields: `allow_paths`, `enabled`, and `confirm` are neither accepted
nor ignored. Project configuration only adds restrictions; it does not disable the catalog.
The library does not search for, open, or load project configuration files itself.

## Catalog and Resolution

Blocks `.env`, `.env.*`, `.ssh`, `.gnupg`, private keys `id_rsa`, `id_dsa`,
`id_ecdsa`, `id_ed25519`, `credentials`, `credentials.json`, `guardrails.toml`, and
suffixes `.pem`, `.key`, `.p12`, and `.pfx`. Public keys such as `id_rsa.pub`
are not blocked by that name. Names and prefixes are compared case-insensitively (ASCII);
no Unicode normalization or substring matching is guaranteed.
Project prefixes respect path component boundaries.

`check_path` checks both requested and resolved paths. It normalizes components and
resolves symlinks, including the existing ancestor of a new destination.
Dangling symlinks or an unavailable root produce an error.

## Shell Checking

```rust
let policy = Guardrails::from_toml("block_commands=['curl']\nblock_paths=['private']")?;
policy.check_shell("git status --short")?;
policy.check_shell_in(Path::new("/workspace"), "cat public.txt")?;
```

`check_shell` parses the lexical subset and checks arguments and redirection
destinations against sensitive paths and project prefixes. It does not resolve file aliases:
`check_shell_in` adds workspace-relative path resolution,
including values after `=` in arguments. Neither executes the command.

The parser accepts words, single/double quotes, supported escapes, comments,
newline or `;` separation, `&&`/`||` chains, pipes, and simple redirections
`<`, `>`, and `>>` with explicit destination. A quoted description is not
reinterpreted as a command. Operators must have complete commands.

Rejects empty input or input over 16 KiB, excessive tokens, unsupported controls,
unclosed quotes, expansions, `$()`/backtick substitutions, globbing,
unsupported continuations, heredocs, backgrounding, and unsupported compound syntax.
Leading assignments, attached numeric descriptors, and ambiguous redirections
are conservatively rejected, even if some forms are valid in POSIX.

## Restrictive Command Catalog

- Blocks `dd`, `shred`, `wipe`, `mkfs`, `mkfs.*`, `fdisk`, and `diskutil`.
- Blocks recursive `rm` and unsupported long options before the `--` separator.
- Blocks `git reset --h*`, global Git options, and `git clean` without unambiguous
  dry-run; accepts only known options for this check.
- Blocks `find` with execution (`-exec`, `-execdir`, `-ok`, `-okdir`) or `-delete`.
- Rejects shell flow control, interpreters `sh`, `bash`, `dash`, `zsh`, `ksh`,
  `fish`, evaluators `eval`, `source`, `.`, `alias`, `trap`, and indirect execution
  via `exec`, `command`, `builtin`, `env`, `sudo`, `doas`, `xargs`, `nohup`,
  `timeout`, `time`, `nice`, `watch`, `parallel`.
- Rejects `cd`, `chdir`, `pushd`, and `popd`: does not simulate changing directories.

The executable name is extracted from the last path component and compared
case-insensitively (ASCII). `alias` and `trap` are blocked even when their
payloads appear harmless; their content is not evaluated to determine permissions.

## Explicit Host Integration

`GuardsPlugin(Arc<Guardrails>)` declares `agent:tool-hooks@1` and registers a
restrictive hook per generation. Before preparing native tools, it checks
shell commands or paths. Removing the plugin revokes the hook; it never grants
authorization or modifies `HostPolicy`.

`native_tools_with_guards` shares policy with adapters. It checks raw and
resolved paths before reading for previews and again on execution.
Search/listing filters entries before recursion or reading; search repeats
the filter before metadata/read. It does not rely solely on the root hook.

`build_native_with_connections_and_guards` requires a host-selected
`Arc<Guardrails>` policy and composes hook and guarded tools together. It does not
load project configuration. Legacy builders and `native_tools` preserve their signatures
and unguided behavior for compatibility; they are not the guarded entry point.
The CLI selects the guarded builder; other hosts must choose it explicitly.

## Explicit Host Loading

`huginn_runtime::guardrails::load_guardrails(workspace)` reads `.huginn/guardrails.toml`
once and returns an `Arc<Guardrails>` snapshot for the guarded builder. The host
chooses the workspace; there is no watcher or automatic reload.
If `.huginn` or the file is missing, it returns the default catalog. An invalid
configuration fails; it does not disable or reduce built-in restrictions.

The loader uses directory capabilities and opens `.huginn` and the file without following
symlinks. Requires regular file, UTF-8, and up to 64 KiB; combines metadata with bounded
reading of 65537 bytes to detect growth. On Unix, `O_NONBLOCK` avoids blocking
when opening a FIFO, which is rejected as non-regular. Failures are not ignored.
The CLI calls the loader once after `prepare_session`, with `session.workspace`,
including in resume mode. It shares the same `Arc` between hook and adapters; there is no hot reload.

## Security Boundaries

Not a sandbox, authorization mechanism, or full shell interpreter.
Does not inspect internal scripts or arbitrary binaries, or implement policy for MCP.
Does not detect all aliases created via hardlinks.
Concurrent substitution between check and I/O remains unresolved: no atomic confinement.
Workspace membership and effects remain the responsibility of the host and adapters;
legacy builders do not enable the policy.
