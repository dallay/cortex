# Restrictive path and shell library

## Status

The host can register `GuardsPlugin` and compose protected native adapters.
The CLI protects run/chat/resume with that composition, with no unprotection flags.
It loads a snapshot from `session.workspace` after creating or recovering the session.

## Explicit use

```rust
use huginn_guards::Guardrails;
use std::path::Path;

let policy = Guardrails::from_toml("block_paths=['private']")?;
policy.check_path(Path::new("/workspace"), Path::new("private/file"))?;
```

The host decides whether to invoke the library. An allowed result does not authorize an
effect nor substitute `HostPolicy`, approval, or workspace validation.

## Configuration from text

`from_toml` accepts TOML text of up to 64 KiB with `block_paths` and `block_commands`.
It supports up to 128 relative paths of 1024 bytes, with no traversal or control characters.
It supports up to 128 additional executables, each non-empty, up to 128 bytes,
with ASCII alphanumeric characters, period, hyphen, or underscore. It compares names
case-insensitively (ASCII) and combines them with the built-in catalog.
It rejects unknown fields: `allow_paths`, `enabled`, and `confirm` are neither accepted
nor ignored. The project only adds restrictions; it does not disable the catalog.
The library does not search for, open, or load a project configuration file.

## Catalog and resolution

It blocks `.env`, `.env.*`, `.ssh`, `.gnupg`, private keys `id_rsa`, `id_dsa`,
`id_ecdsa`, `id_ed25519`, `credentials`, `credentials.json`, `guardrails.toml`, and
suffixes `.pem`, `.key`, `.p12`, and `.pfx`. Public keys such as `id_rsa.pub`
are not blocked by that name. Names and prefixes are compared case-insensitively (ASCII); no Unicode normalization or substring matching is promised.
Project prefixes respect boundaries between path components.

`check_path` checks both the requested path and the resolved one. It normalizes components and
resolves links, including the existing ancestor of a target that is still new.
Dangling links or an unavailable root produce an error.

## Shell checking

```rust
let policy = Guardrails::from_toml("block_commands=['curl']\nblock_paths=['private']")?;
policy.check_shell("git status --short")?;
policy.check_shell_in(Path::new("/workspace"), "cat public.txt")?;
```

`check_shell` analyzes the lexical subset and checks arguments and redirection
targets against sensitive paths and project prefixes. It does not resolve file
aliases: `check_shell_in` adds path resolution relative to the workspace,
including values after `=` in arguments. Neither executes the command.

The parser accepts words, single/double quotes, supported escapes, comments,
separation by newline or `;`, `&&`/`||` chains, pipes, and simple redirections
`<`, `>`, and `>>` with an explicit target. A quoted description is not
reinterpreted as a command. Operators must have complete commands.

It rejects empty input or input over 16 KiB, excess tokens, unsupported control characters,
incomplete quotes, expansions, `$()`/backtick substitutions, globbing,
unsupported continuations, heredocs, background execution, and unsupported compound syntax.
Initial assignments, attached numeric descriptors, and ambiguous redirections
are rejected conservatively, even when some forms are valid in POSIX.

## Restrictive command catalog

- Blocks `dd`, `shred`, `wipe`, `mkfs`, `mkfs.*`, `fdisk`, and `diskutil`.
- Blocks recursive `rm` and unsupported long options before the `--` separator.
- Blocks `git reset --h*`, global Git options, and `git clean` without unambiguous
  dry-run; it only accepts the known options of that check.
- Blocks `find` with execution (`-exec`, `-execdir`, `-ok`, `-okdir`) or `-delete`.
- Rejects shell control flow, interpreters `sh`, `bash`, `dash`, `zsh`, `ksh`,
  `fish`, evaluators `eval`, `source`, `.`, `alias`, `trap`, and indirect execution
  through `exec`, `command`, `builtin`, `env`, `sudo`, `doas`, `xargs`, `nohup`,
  `timeout`, `time`, `nice`, `watch`, `parallel`.
- Rejects `cd`, `chdir`, `pushd`, and `popd`: it does not attempt to simulate changing directories.

The executable name is taken from the last component of its path and compared
case-insensitively (ASCII). `alias` and `trap` are blocked even when their
payloads look harmless; their content is not interpreted to decide permissions.

## Explicit host integration

`GuardsPlugin(Arc<Guardrails>)` declares `agent:tool-hooks@1` and registers one
restrictive hook per generation. Before preparing native tools, it checks
shell commands or paths. Retiring the plugin revokes the hook; it never grants
authorization or modifies `HostPolicy`.

`native_tools_with_guards` shares the policy with the adapters. It checks
raw and resolved paths before reading for previews and again when executing.
Search/listing filters entries before recursion or reading; search repeats
the filter before metadata/reading. This way it does not depend solely on the root hook.

`build_native_with_connections_and_guards` requires an `Arc<Guardrails>` policy
selected by the host and composes the hook and the protected tools together. It does not
load project configuration. The legacy builders and `native_tools` keep
their signatures and behavior without a policy for compatibility; they are not the protected
entry point. The CLI selects the protected builder; other hosts must choose it explicitly.

## Explicit host loading

`huginn_runtime::guardrails::load_guardrails(workspace)` reads
`.huginn/guardrails.toml` once and returns an `Arc<Guardrails>` snapshot for the
protected builder. The host chooses the workspace; there is no watcher or automatic reload.
If `.huginn` or the file is missing, it returns the default catalog. An
invalid configuration fails; it does not disable or reduce the built-in restrictions.

The loader uses directory capabilities and opens `.huginn` and the file without following
links. It requires a regular file, UTF-8, and up to 64 KiB; it combines metadata with a
bounded 65537-byte read to detect growth. On Unix, `O_NONBLOCK` avoids
blocking when opening a FIFO, which is rejected as non-regular. Failures are not ignored.
The CLI calls the loader once after `prepare_session`, with `session.workspace`,
also on resume. It shares the same `Arc` between the hook and adapters; there is no hot reload.

## Security limits

It is not a sandbox, an authorization mechanism, or a complete shell interpreter.
It does not inspect the inside of arbitrary scripts or programs, nor does it implement a
policy for MCP. It also does not detect all hardlink-based aliases.
Concurrent substitution between checking and I/O remains unresolved: there is no
atomic confinement. Workspace membership and effects remain the
responsibility of the host and its adapters; legacy builders do not activate the policy.
