# Experimental WASM Rust outline

This experiment implements DALLAY-733 / [#506](https://github.com/dallay/cortex/issues/506).
It does not accept WASM as a production runtime or stabilize an external SDK.
[ADR-0013](adr/0013-authorable-plugin-contract-and-loading.md) still applies.

## Build and run

The repository toolchain includes `wasm32-unknown-unknown`. Install the pinned
component packaging CLI once:

```sh
cargo install --locked wasm-tools --version 1.262.0
just huginn-wasm-build
cargo build --locked -p huginn --features experimental-wasm
target/debug/huginn --provider mock --json --workspace . \
  --experimental-wasm-outline target/rust-file-outline.component.wasm \
  run 'huginn-mock: rust_file_outline {"path":"crates/huginn/runtime/src/tools.rs"}'
```

The host and component are separate artifacts. Build the host once, then replace
the explicitly selected component artifact without rebuilding or relinking the
host. The host never compiles Rust extensions at startup. It validates and
compiles the supplied WebAssembly component before registering its tool.

Both the Cargo feature and the invocation flag are required. Without the flag,
no component is opened, compiled, instantiated, or registered. The flag is only
accepted with `run` or `resume --prompt`; interactive loading is not supported.
No package installation, directory discovery, reload, or marketplace is provided.

## Capability and trust boundary

The experimental contract is
`crates/huginn/plugin-api/wit/experimental-rust-outline/outline.wit`, package
`huginn:rust-outline@0.1.0`. Its only host import is a borrowed `snapshot` resource
with `size` and bounded `read` methods. There is no resource constructor, pathname
import, or WASI filesystem, process, clock, random, environment, or network linker.
Unsupported imports or incompatible exports are rejected before publication.

The host pins the selected workspace directory when constructing the plugin,
before guest execution. Calls must use that selected workspace, and snapshot
reads retain its directory capability rather than reopening the workspace name.
Replacing the workspace root or one of its ancestors does not redirect reads.
The model proposes a workspace-relative path. The host checks the existing read
policy during preparation and again before taking the snapshot. It rejects
traversal, sensitive/project-blocked paths, nonregular files, and symlinks in
every path component, including symlinks that remain inside the workspace.
Directory-capability-relative opens prevent a check-then-open symlink escape.
An authorized file snapshot does not authorize neighboring files or external
modules. The component never receives the pathname.

Read permission is not interactive effect approval: this tool preserves the
existing read policy and requests no additional approval by default. The plugin
does not own host policy and cannot grant access through tool metadata.
Native plugins remain trusted in-process code; this experiment does not change
their security boundary or migrate them to WASM.

Each invocation creates a new store, instance, resource table, and snapshot.
The component receives a borrowed resource, not an owned grant that can survive
the call. Resource IDs are scoped to that store, and fabricated guest handles
are rejected by the canonical ABI/resource table. Nothing transfers an instance
or its resource IDs between invocations.

## Output contract

`rust_file_outline` returns UTF-8 JSON with:

- `complete`: whether the supported syntactic analysis completed without limits.
- `declarations`: names, kinds, optional containers, original source signatures,
  and source ranges.
- `diagnostics`: explicit reasons for syntax errors or incomplete output.

Ranges use half-open UTF-8 byte offsets and one-based line/Unicode-scalar column
positions. CRLF is preserved in the original source. Functions, structs, enums,
unions, modules, traits, implementations, type aliases, constants, statics, and
associated methods/types/constants are supported. Inline containers are visited;
function bodies and macro bodies are not traversed. An external `mod name;` is
reported but never opened. Syntax errors return no declarations, `complete`
set to `false`, and a diagnostic. Limits retain whole declaration records,
never broken JSON. Type resolution, macro expansion, references, and project
indexing are not supported. `complete` is not a claim of semantic analysis.
The guest resolves sorted declaration endpoints in a single forward source scan,
including nested ranges that are not visited in byte order. Output admission
reserves worst-case coordinate widths and can stop before the exact byte cap.

The host checks the output size, structure, completeness diagnostics, and source
range/signature consistency before passing it into the agent continuation.
Extracted source and guest-generated output remain untrusted model input.
Structural checks do not authenticate the semantic meaning of guest metadata.

## Resource limits and lifecycle

Initial experimental limits are concrete policy values, not performance claims:

- Component input: 16 MiB, regular file only; final symlinks are rejected.
- Source snapshot: 1 MiB, UTF-8 for analysis.
- A host read: 64 KiB; cumulative returned bytes: 1 MiB per invocation.
- Guest linear memory: 64 MiB per memory, at most two memories.
- Tables: at most four, each at most 10,000 elements; at most 16 core instances.
- Native WASM stack: 512 KiB.
- Execution: 100 million fuel units, yielding every 10,000 fuel units.
- Invocation deadline: five seconds, including instantiation and output checking.
- Output: 48 KiB; at most 1,000 declarations.
- Concurrency: one invocation per loaded adapter; excess calls fail immediately.

Fuel is runtime-specific, not an instruction count or a latency guarantee.
The deadline can be observed when instrumented guest execution yields. The
host does not detach a blocking worker: cancelling or dropping the async call
destroys its store and resources. Registry generation revocation also drops the
in-flight future and rejects retained tool handles. Success, error, trap, fuel
exhaustion, memory failure, cancellation, and unload release the invocation.
Read quota/range violations trap rather than returning an ignorable error.
Component files are opened nonblocking on Unix before their regular-file check,
so selecting a FIFO does not hang startup.

Component validation/JIT compilation happens in host startup, before invocation
fuel limits apply. The component-size limit bounds its input, not a separate
OS process's memory or CPU. This is a local opt-in experiment, not a claim of
production-safe installation or hostile-compilation containment.

The `activity()` counters expose active invocations and snapshot import counts,
without paths or contents, to diagnose entry, cancellation, and cleanup.
Registration and rollback use the existing `ToolRegistrationService`, leases,
and supervisor transactions, not a parallel plugin registry.

## Verification

```sh
just huginn-wasm-test
cargo test --locked -p huginn-core -p huginn-application -p huginn-plugin-api
cargo test --locked -p huginn-runtime --features experimental-wasm \
  --test tool_authoring --test tool_lifecycle --test coding_workflow
cargo test --locked -p huginn --features experimental-wasm --test guardrails
```

Real-component tests rebuild the guest separately and package it using pinned
Rust development dependencies. They do not require a globally installed
`wasm-tools`, use checked-in binaries, or silently skip a missing target. Their
guest build uses a separate target directory and removes host coverage flags.
The pure parser also runs natively. Generated ABI bindings live in the small
`huginn-outline-bindings` dependency so external generated export macros retain
their hygiene; all workspace crates retain `unsafe_code = "deny"`, with no
handwritten unsafe blocks or lint exceptions.

Independent WAT components test active non-cooperative execution, traps,
resource exhaustion, fabricated handles, and invalid output. A fast successful
outline does not itself demonstrate effective cancellation. Tests wait until
the guest actually calls a snapshot import before cancelling or unloading it.

Full acceptance additionally requires an actual LLM to call this tool and use
its result for a navigation task. A mock continuation is deterministic evidence,
not a substitute for that validation. Missing credentials or unavailable models
must remain a visible acceptance blocker. Never record keys, authenticated URLs,
or sensitive source in evidence. Production promotion requires a separate review
of the measured evidence and remaining limitations.

## Recorded local evidence

On macOS with the repository toolchain, the independent component build and both
headless validations passed. The host binary was built first, then the guest was
rebuilt separately; the host hash remained unchanged through that build and both
executions.

- Host SHA-256: `31f34cc85b649a89226b569989440ebd338fba318ee00652edac6e0c3d22b4aa`.
- Component SHA-256: `8e0d911a9daa7b23bbeea00a31210531472dd4cbd3f47eb05b33782fb4386b47`.
- Component size: 1,193,559 bytes.
- Deterministic run: actual component result reached the mock-model continuation.
- Actual provider: the configured OpenAI-compatible route, model alias
  `auto/best-coding-fast`, called `rust_file_outline` once and used its result to
  locate `Scheduler` at lines 4–6, its `Queue::next_ready` implementation at lines
  9–11, and `drain_ready` at lines 13–17 in a synthetic fixture. The run completed
  in 39.76 seconds, including startup and provider calls; this is not a benchmark.

No credentials, authenticated endpoint values, or private repository source were
recorded. These hashes identify the local artifacts used, not release artifacts.
Full local CI remains unverified because Trivy is unavailable. The existing
OS-secret-store integration test remains explicitly ignored in the runtime suite.

The guest-position regression uses a 900 KB comment prefix followed by one or
100 declarations, both with and without a newline after the prefix. Before the
fix, the one-declaration control passed but the 100-declaration component failed
under the existing execution budget. Sorted endpoints and disjoint source scans
make both cases pass without changing fuel, memory, deadlines, or output limits.
