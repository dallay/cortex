# Agent validation record

Date: 2026-10-06. Implementation host: macOS, Rust 1.89.
Linux fixture host: ARM64 container, Rust 1.99 (existing stable toolchain).

## Cadena de hooks, registro y guards — 2026-10-27

**Estado: Ready para abrir la cadena de PRs; no es aprobación de merge.**
Código integrado verificado: `6236d57`, sobre main `cb92f8a`. Doce ramas locales
en `feature-branch-chain`, con referencia `integration/huginn-guards`.
Cada capa cumple su presupuesto: 400 líneas, salvo las excepciones autorizadas
del scanner, 455/470, y shell, 786/800. Los PRs dependientes apuntan a su rama padre.

- `just ci-local`: nueve etapas aprobadas en 524 segundos; incluye tests Rust,
  frontend, documentación y 90 E2E con imagen reconstruida desde el candidato.
- `RUST_TEST_THREADS=1 cargo llvm-cov --workspace --all-features --fail-under-lines 87 --lcov`:
  aprobado; 37068 de 41326 líneas, **89,70 %**, por encima del umbral de 87 %.
- Semgrep 1.126.0 con los tres rulesets de CI: cero findings activos, 233 targets.
  Se mantienen visibles cuatro avisos de parsing parcial preexistentes en
  PowerShell de `release.yml`; no se afirma cobertura completa del scanner.
- Siete excepciones nuevas exactas y siete históricas separadas, con controles
  positivos, negativos y supresiones desactivadas. Diez tests Python aprobados.
  Los cambios aislados de los scripts activan CI: 290 tests Node de metadatos
  aprobados, incluidos los dos casos de clasificación añadidos tras actualizar main.
- `cargo audit`: cero vulnerabilidades, con avisos de mantenimiento. Gitleaks:
  rango completo de la cadena desde `cb92f8a`, catorce commits sin secretos.
- Trivy 0.76.1, con filtros de CI `HIGH,CRITICAL` e `ignore-unfixed`: cero findings
  en ambos lockfiles y ambos Dockerfiles. El snapshot se analizó sin usuario root
  ni escritura en el proyecto; no se extiende el resultado fuera de esos filtros.
- Revisiones independientes aprobaron las unidades y sus correcciones. Los límites
  heredados de notas/resultados y las dos fronteras del lexer tienen regresiones
  que fallaron antes de corregirlos y pasaron después. No se eliminaron pruebas.

Se conservan los límites de código nativo confiable, ausencia de sandbox y riesgo
residual de sustitución concurrente de archivos. ADR-0016 sigue `Proposed`.
Los quality gates remotos, incluidos Sonar/CodeQL y la revisión humana, deberán
ejecutarse al publicar: esta evidencia local no los sustituye.
No se ha realizado push ni se han abierto PRs. El cierre posterior a `6236d57`
solo incorpora esta evidencia documental, sin modificar código ejecutable.

## Skills and prompt templates S1 acceptance (DALLAY-717) — 2026-10-26

Automated integration acceptance for the S1 skills/template seam
([#432](https://github.com/dallay/cortex/issues/432)). Tested base commit:
`5ce0964` plus this change. Host: macOS 27.2 (arm64), Rust 1.99.1. All runs use
the offline `--provider mock`, temporary home/project roots and no network or
credentials. This is **not** human acceptance, terminal certification or
release readiness.

The fixture drives the real `huginn` binary with a global `lint`, a global
`unused`, a project `review` that overrides a global `review`, and the
parameterized project template `audit`. The mock follows `huginn-mock:` lines
(see the Huginn README) so it can request `load_skill`, `write_file` and `shell`
like an instruction-following model; every request still crosses HostPolicy.

| Criterion | Entry point | Evidence |
| -- | -- | -- |
| AC-001 | `skills_acceptance::template_lazy_load_and_explicit_skill_compose_across_run_line_and_resume` | Template renders; the mock lazily loads only `lint`; `/skill:review` carries the project body in its envelope; unused/shadowed bodies never reach session data |
| AC-002 | `skills_acceptance::malicious_skills_cannot_escalate_effects_or_escape_the_workspace` | A lazily loaded skill's write is denied (`approval_resolved approved:false`); a line-mode shell request is denied by `n`; `allowed-tools` frontmatter grants nothing; a granted `../` write fails and creates no file |
| AC-003 | `skills_acceptance::oversized_stale_and_invalid_resources_fail_closed_and_resume_cleanly` | Oversized load fails without storing the body; a stale identity after an edit fails and reports the catalog change; invalid and dangling-symlink skills are skipped with catalog diagnostics; resume replays no tool call |
| AC-004 | the AC-001 test, plus `tui::skills_templates_denial_and_cancellation_release_the_tui_and_resume_cleanly` | `run`/resume and `--line-mode` store identical user turns with valid NDJSON; TUI template, explicit skill, Esc denial and Ctrl+C cancellation at a pending approval release the composer; a restarted process replays nothing. The shell approval is identified by its preview, and Ctrl+C must end the turn with `error: operation cancelled` and no `approval_resolved`, `tool_finished` or `turn_finished` for it, while a denial resolves the approval and continues |
| Regression | `skills_acceptance::clean_workspace_without_resources_keeps_plain_turns_working` | Plain turns are unchanged; unknown skills or templates fail before a session is stored |

In-process cancellation of an explicit skill load before turn start stays covered by
`coding_workflow::explicit_skill_revalidation_and_inflight_cancellation_fail_before_turn_start`.
Two mutation checks confirm the tests can fail: making HostPolicy approve everything
failed the AC-002 test, and making Ctrl+C stop cancelling the turn failed the AC-004
TUI test, which then observed `approval_resolved` where cancellation is required.
Both mutations were reverted.

Commands and outcomes:

- `cargo test -p huginn --test skills_acceptance`: 4 passed.
- `cargo test -p huginn --test tui`: 13 passed (1 new).
- `just huginn-test`, which includes `just huginn-architecture`: 534 passed,
  0 failed. 2 tests were ignored: legacy-binary interop (needs
  `CORTEX_AGENT_LEGACY_BIN`) and the OS secret-store test.
- `cargo clippy -p huginn -p huginn-runtime -p huginn-application --all-targets --all-features -- -D warnings`
  and `cargo fmt --all -- --check`: passed.

`just ci-local` ran on this host and failed at stage 5/9, outside this change:
`rook --test di_tests::test_sqlite_secret_storage_through_api_key_auth` reports
`secret path would escape database directory`. In `apps/rook/src/di.rs` the
database path falls back to its non-canonical form when the file does not exist
yet, then is compared against a canonicalized parent; on macOS `/var` is a
symlink to `private/var`, so the comparison fails and Linux runners do not see it.
`cargo test --workspace --no-fail-fast` reported 1750 passed and the same test
failing twice (parent and child process); nothing else failed. This change
contains no `apps/rook` file and `apps/rook` depends on no `huginn` crate. The
GitHub Actions workflow on Linux passed for the PR head.

Not verified: Linux and Windows daily use, native terminal products, SSH/tmux, real
models and human approval usability. Those remain with
[DALLAY-665](https://linear.app/dallay/issue/DALLAY-665) (#307),
[DALLAY-668](https://linear.app/dallay/issue/DALLAY-668) (#332) and
[DALLAY-634](https://linear.app/dallay/issue/DALLAY-634) (#280), which this
record does not close.

## Hexagonal/plugin lifecycle refactor — 2026-10-11

[ADR-0012](adr/0012-hexagonal-application-and-plugin-lifecycle.md) is implemented
with an application crate, instruction port, host-owned authorization,
generation-scoped native/MCP tool plugins and a Cargo architecture gate.

- Focused five-package Huginn suite with all features and `--test-threads=1`:
  **108 passed, 0 failed, 1 ignored**. The ignored legacy-binary interoperability
  test needs `CORTEX_AGENT_LEGACY_BIN`; it is not claimed as verified.
- All-target/all-feature Clippy with warnings denied, formatting, Rustdoc with
  warnings denied, focused Markdown lint and architecture guard: passed.
- The architecture guard includes a negative control with a renamed optional
  dependency and runs in CI check, `just ci-check`, `just ci-local` and
  `just huginn-test`.
- New contract/lifecycle tests cover an entirely in-memory loop, exact host grants,
  cancellation, unavailable presentation, staged visibility, failed activation,
  stale handles, generation replacement and provider factory extension.
- MCP rollback tests cover a valid tool followed by an invalid schema, a successful
  server followed by startup failure, and removal with a retained tool handle.
  Both standalone discovery and production plugin paths preserve registry snapshots.
- Local sockets require execution outside the filesystem sandbox. A parallel
  repeat encountered two Python MCP initialization timeouts; sequential validation
  passed. New MCP lifecycle tests are kept in their own integration-test binary
  to avoid increasing concurrency in the existing transport timing suite.

This is focused macOS evidence. The full Rook/dashboard/Docker workspace gate and
Linux terminal certification were not rerun for this refactor.

## Ratatui vertical slice — 2026-10-10

The first [ADR-0010](adr/0010-ratatui-plugin-first-interactive-terminal.md) presentation plugin is implemented. Dependency pins: Ratatui 0.30.2, Crossterm 0.29.0; Tokio is resolved by `Cargo.lock`. Evidence here is automated macOS PTY evidence, **not** native Terminal.app/iTerm2, SSH, tmux, Linux or Windows certification. `--line-mode` remains the deliberate recovery route. The slice was rebased onto the Huginn rename (`agent-*` → `huginn-*`; service ids stay `agent:*`); `session_ownership` pins its lease owner to `--line-mode` because interactive `resume` opens Ratatui by default.

- `cargo test -p huginn-presentation --all-features --tests`: 8 deterministic state/plugin tests and 7 PTY tests passed. Coverage includes UTF-8 multiline editing, one-time transcript synchronization, 10,000 synchronous text transitions plus critical events without a renderer, full large approval payloads, challenge freshness/cancellation, command/renderer unload and re-registration, missing dependencies, delayed stream typing/cancellation, stale/paste confirmation rejection, fresh approval, normal/panic/failed-init restoration, synchronous connection drop, unload→immediate reopen and drop→replacement input.
- `cargo test -p huginn --tests`: 10 existing unit tests, 5 CLI tests and 2 new routing/PTY tests passed. The actual binary's default chat and interactive resume use the inline driver, save multiline prompts and preserve session messages. Non-TTY default failure and explicit piped line recovery passed. Existing JSON/run/doctor/sessions/manual-compaction checks remained green.
- `cargo clippy -p huginn-presentation -p huginn --all-targets --all-features -- -D warnings`: passed. Formatting and focused Markdown lint passed. Post-review verification ran all 15 presentation tests and focused all-features Clippy against the final input-lease implementation.
- Minimized cursor-query failure: an EventStream reader running while stock Ratatui inserted initial inline history caused cursor-position timeouts in two PTY tests. The driver now uses bounded zero-timeout Crossterm polling without a competing input thread; those tests pass. No custom terminal or copied upstream code was introduced. This is relevant to stock Ratatui #2640 and is tracked with the broader [DALLAY-665 compatibility gate](https://linear.app/dallay/issue/DALLAY-665).
- Independent review found End→Up navigation, asynchronous connection-drop restoration and unload cleanup timing defects. Regressions failed before fixes and now pass. Supervisor-owned cleanup waits for terminal destruction; connection destruction restores raw mode synchronously.
- Follow-up review found an input-revocation race because task abort is asynchronous. Both input paths now share the restoration lock and check generation activity before poll/read. Final focused review confirms this structurally addresses the race. The added drop→reopen→input PTY test passed before and after, so it is coverage, **not** a deterministic RED→GREEN reproduction of that race. `cargo tree` verified no presentation/Ratatui/Crossterm dependency in core/runtime/Rook.
- Review follow-ups fixed with regression tests: Unicode bidi controls filtered in `safe()`; `UiState` event log bounded to non-text authoritative transitions; contribution command validation (invalid/reserved/same-generation duplicate/cross-generation collision) covered deterministically.
- Review round 2 (scoped REQUEST CHANGES, no P0/P1) addressed with tests: out-of-scope Rook doc deletions reverted out of the PR; approval numbering (`request #N`) and `Effect:` summaries restored in the TUI modal through the shared `huginn-core::approval` model (line adapter rewired to the same functions, output byte-identical); contribution surface extended with a contributed failure heading rendered with error severity and scoped as status-only in the spec; composer title carries the Huginn identity with a PTY regression guard; O(tail) re-render cost noted in code and tracked with the DALLAY-665 load/latency measurements.
- Known advisory: `codecov/patch/backend` reports ~78% against an 85% target. The Ratatui driver is verified by PTY tests that drive the binary as a subprocess, which `llvm-cov` cannot attribute; in-process presentation-crate coverage is ~86% regions. Overall `codecov/patch` passes and the repository gate (`just ci-local`) sets no coverage threshold.
- Semgrep source scan before generation passed. The first dependency selection (Ratatui 0.29) exposed an `lru` advisory; selecting stock Ratatui 0.30.2 resolved it. The subsequent supply-chain scan reported no `Cargo.lock` findings, but 75 findings in existing non-Rust lockfiles; this is not a repository-wide clean security scan.

Remaining DALLAY-665 evidence: semantic terminal-emulator scrollback/resize checks (including 100 rapid transitions), 10k-line/100 tok/s/10× burst and input-latency measurements, native-terminal versions, Linux, SSH/tmux, SIGHUP and subprocess handoff. Human approval usability and real-model TUI smoke remain unvalidated. The PTY harness responds to cursor queries as a simulated xterm; it does not certify xterm or any actual terminal product.

### DALLAY-664 repository gate

`just ci-local` completed successfully on 2026-10-10 in 496 seconds: Markdown,
formatting, workspace Clippy/check, 936 Rust tests (including doctests), 179 Vitest
tests, documentation, audit and Docker-backed browser E2E. Browser E2E reported
83 passed and 7 skipped across Chromium/Firefox/WebKit. The final ownership-review
change and added replacement-input PTY test also passed the subsequent focused
presentation tests and all-features Clippy; the full gate's initial lint stages
preceded that change. These dashboard E2E results are not Linux TUI evidence.

Warnings: Vite reported a future native-config import-extension warning;
`cargo audit --no-fetch` reported the existing yanked `chacha20` 0.10.0 warning.
The audit stage remains non-blocking in the repository runner. Skipped browser
tests and existing non-Rust supply-chain findings are not claimed as validated.

## Automated evidence

The deterministic suite covers service lifecycle and rollback, approved coding
changes followed by shell tests, denied edits, stale diffs, workspace containment,
fragmented SSE, durable session recovery, compaction, instruction scopes, process
timeouts, HTTP adapter behavior, MCP discovery/failure and CLI JSON/resume.
The 21 agent tests passed on macOS and in a Linux ARM64 container without network
access. The CLI also completed an offline read-only turn over Cortex itself.
The full workspace gate passed Markdown, formatting, Clippy, compile, Rust tests,
175 Vitest tests and documentation; the final Docker E2E result is pending below.

The E2E Dockerfile used an obsolete pnpm --include flag. It now installs the frozen
lockfile with dev dependencies included by default. The E2E runner also propagates
Docker build failures through its log pipeline.

The repository's cargo audit step is non-blocking (uses `|| true`). It reports four
advisories in existing lockfile packages: crossbeam-epoch 0.9.18, h2 0.4.14,
quinn-proto 0.11.14 and rustls 0.23.40, plus warnings for anyhow and chacha20.
Those versions were already present; no dependency upgrades were included in this
feature. Passing the configured CI gate does not mean the audit is clean.

Semgrep tools were unavailable for the earlier implementation checks; current
focused MCP source and test scan completed with no findings.

## Remaining acceptance

- Real-model coding and restart/resume smoke: **passed on macOS 2026-10-07**;
  see the record below. Interactive approval usability still needs human daily use.
- Linux daily use: deterministic coding and MCP fixtures passed; repeat the
  interactive workflow with a real model on a Linux host. **Checklist and
  smoke harness are ready** — see "Linux daily-use record" below. Five `ok`
  sessions across at least two repositories are the closing bar.
- ~~Rook: verify structured streaming tool-call arguments before declaring
  support.~~ **Closed 2026-10-06 — not supported as a backend.**
  See "Rook compatibility result" below.
- Public packaging and release tags follow the personal MVP.

## Real-model smoke record — 2026-10-07

Host: macOS, Rust 1.89.0. Endpoint: `https://omnirouter.ahome.quest/v1`.
Requested model route: `auto/best-coding-fast`; the underlying model was not
identified. This result establishes the tested route's compatibility for this
workflow, not every model or endpoint.

A temporary terminal harness drove the actual CLI in a disposable Python
workspace. It answered each displayed approval prompt with `y`; it did not use
`--allow`. The temporary API credential was provided in the process environment,
never in configuration, fixture files or this record.

- Coding: two `read_file` calls inspected a broken addition function and its
  unittest. `edit_file` displayed a diff and received approval before replacing
  subtraction with addition. A separately approved `shell` call ran
  `python3 -m unittest -v` successfully. An independent test execution also passed.
- Interruption: a separately approved shell command waited 30 seconds before
  creating a marker file. SIGINT after approval interrupted the turn; the saved
  session retained its interrupted state and the marker was absent.
- Restart/resume: a new CLI process resumed the interrupted session, completed
  a read-only inspection and retained an unknown-completion recovery result for
  the interrupted call. The marker remained absent; the command was not replayed.
- Regression checks: all 28 agent tests passed (4 kernel, 16 coding workflow,
  6 transport, 2 CLI). `doctor` now reports Rook as `unsupported`, matching the
  compatibility record. The Linux checklist uses `chat` for individual approvals,
  explains restarting after cancellation and requires observed compaction.

The user authorized omitting unavailable Semgrep tools for that change. Real-model
compaction, human approval UX and Linux daily use remain unvalidated by this smoke.

## DALLAY-631 acceptance session — 2026-10-09

Host: MacBook Pro M2 Max, 32 GB unified memory. Ollama server version: 0.40.2.
An isolated test directory, SQLite database and disposable workspace were used
under `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/dallay631-ollama.PhjPZU`.
The workspace contained only a README fixture and no MCP server config. No API
credential was needed; the temporary config set `api_key_env = ""`.

### Qwen2.5-Coder baseline

With `qwen2.5-coder:14b`, a direct text completion and agent `doctor` passed. In
both direct tool-schema and actual CLI attempts, the model returned tool-call-like
JSON in normal assistant `content`, rather than structured `tool_calls`. Thus the
agent correctly did not execute the requested tool. This was not counted as
acceptance of tool calling or compaction.

### Qwen3-8B structured tool compatibility

Model used: `qcwind/qwen3-8b-instruct-Q4-K-M:latest`, architecture Qwen3, 8.2B,
Q4_K_M, Ollama reports tools capability. It was loaded fully on GPU with a 32K
runtime context. On the exact minimal `read_file` tool request:

- OpenAI-compatible `/v1/chat/completions`, `stream=false`: **PASS** — response
  had `finish_reason="tool_calls"` and `message.tool_calls[0]` named `read_file`
  with `{"path":"README.md"}`.
- Native `/api/chat`, `stream=false`: **PASS** — response had `message.tool_calls`
  with the same tool name and object arguments.
- OpenAI-compatible `/v1/chat/completions`, `stream=true`: **PASS** — SSE emitted
  `choices[0].delta.tool_calls`; tool-call arguments were fragmented across events
  as allowed by the streaming protocol.
- Actual Cortex CLI `run` in the isolated workspace: **PASS** — it invoked
  `read_file`, printed the README heading `Marea Demo`, and returned without
  changing the workspace. This confirms the streamed structured tool path works
  through the current Cortex adapter for this model/runtime combination.

### Compaction and persisted-original evidence

A single CLI session was resumed across multiple processes. The temporary config
used a low `context_tokens = 4096` and `max_output_tokens = 1024` to reach the
configured compaction threshold. The model produced a coherent summary preserving
the Marea task and key decisions, and subsequent resumed turns could answer from
that summary (for example, identify Marea and `ValueError` for an empty list).
SQLite inspection confirmed the persisted `summary_through` advanced to 12 and
`summary` was present. Earlier original messages, including a complete assistant
tool call and its corresponding tool result, remained in `messages`; the workspace
README hash remained unchanged.

**Limitations observed:** with the small budget, the model summary did not retain
all four pending test cases: after compaction the model reported insufficient
context for those details. Therefore this demonstrates that compaction triggered,
original messages/tool pair were retained, and the model continued using salient
summary facts; it does **not** establish perfect semantic retention of every detail.
When a repeated follow-up ran into the intentionally tight 4096-token budget, the
loop correctly reported `context cannot fit safely` rather than sending an unsafe
request. Raising the temporary test budget to 8192 allowed the model to continue
from the summary. No corrupt session or workspace mutation was observed. The
separate automated `failed_compaction_preserves_original_history_and_stops` test
continues to cover summary failure rollback/preservation.

### Interactive approvals: not accepted

An attempt to run interactive `chat` under the shell tool failed because it did
not provide a usable TTY; the CLI correctly rejected non-terminal stdin. A PTY
harness attempt timed out before a model response, and no approval was granted.
The disposable workspace README remained unchanged. Thus no real-model human
approval was exercised for file edits or shell commands. MCP startup and MCP tool
approval were not tested because no local MCP server/config was provided.

Conclusion: Ollama/Qwen3-8B is compatible with Cortex's streamed structured tool
calls in this setup and can trigger/continue after compaction, with the noted
summary-fidelity limitation. The issue's approval-UX acceptance remains open until
file-edit, shell, MCP-start and MCP-call prompts are manually examined, including
denials and fresh-process behavior.

### 2026-10-09 follow-up — approval numbering and `/compact`

The local acceptance session surfaced an approval UX gap: a second approval in
a turn did not indicate its position in the sequence. The terminal policy now
numbers each approval and prints single-line `Action:` / `Effect:` headers
above the full preview. It does not claim a later request is different unless
that property is actually established. The CLI accepts `/compact` (with
`/summarize` as an alias) with explicit confirmation. Manual compaction selects
the latest complete finalized turn; automatic compaction preserves the latest
complete turn while trimming older history. Both share the same summary
request, budget/result validation, persistence, and event-emission helper.
The command calls the registered loop, reuses the `Output` event sink, and
propagates Ctrl+C cancellation.

Regression coverage now includes runtime compaction across 1, 2, and 3
complete turns with tool-call/result groups, cancellation without persisting a
summary, exact approval header text for repeated identical actions, and PTY
integration tests for accepted/declined `/compact` confirmation in JSON mode.
These use the mock provider and do not replace manual terminal acceptance of
repeated approvals or real-model compaction; both remain pending.

Focal checks: `cargo fmt --check`; runtime `stream_persistence` (4 passed),
`coding_workflow` (27 passed), `transports` (6 passed); CLI unit tests (7 passed)
and CLI integration tests (4 passed); `cargo clippy -p huginn
-p huginn-runtime --all-targets -- -D warnings`; focused Markdown lint (0 issues);
`git diff --check`.

## Integración nativa de políticas, sin activación CLI (B9)

La unidad añade plugin restrictivo, checks de adaptadores y builder protegido
explícito. Conserva el registro existente y las firmas de composición legacy.
Las pruebas de `huginn-guards` incluyen lifecycle del plugin; runtime `guardrails`
cubre bloqueo antes de previews y búsqueda recursiva con `.env` y `.ENV`.
Checks focales: guards, runtime `guardrails`, `tool_authoring`, `tool_lifecycle`,
`mcp_lifecycle`, `coding_workflow`, clippy de guards/runtime, formato y arquitectura.
`just huginn-test` incluye guards, plugin-api y reference-extension.
Esto no certifica carga de archivos, activación run/chat/resume ni CI/cobertura
global. Esas validaciones corresponden a las capas y al extremo integrado siguientes.

## Cargador explícito de políticas (B10)

Runtime `guardrails` conserva las dos regresiones nativas de B9 y añade carga
aditiva acotada, rechazo de configuración inválida/no regular y, en Unix, enlaces
y FIFO sin bloqueo. El FIFO se crea únicamente en una fixture temporal propia.
La carga es una API explícita del host; esta unidad no activa la CLI ni sustituye
las comprobaciones del extremo integrado, cobertura Rust o quality gates remotos.

## Activación CLI y verificación local (B11)

Las secciones B9/B10 anteriores describen snapshots históricos, no el estado
actual del binario. B11 carga la política desde la sesión preparada y selecciona
el builder protegido antes de run/chat/resume, línea o TUI. No cambia credenciales,
MCP, providers, instrucciones ni persistencia; conserva la autoridad de `HostPolicy`.

TDD local: antes de activar la entrada protegida, cinco tests CLI fallaron y uno
pasó: hubo efectos solo dentro de fixtures, búsqueda sensible y política ignorada.
Tras el wiring original, los seis aprobaron, incluido rechazo de alias/trap con
payloads inocuos, bloqueo pese a grants, política inválida, búsqueda y resume.

Comandos reproducibles ejecutados con caché Cargo externa:

```sh
cargo test -p huginn --test guardrails --test cli --test skills_acceptance \
  --test skill_invocation --test skills_cli --test prompt_templates \
  --test providers_recovery --test session_ownership --locked -- --test-threads=1
cargo test -p huginn --test tui --locked -- --test-threads=1
```

Aprobaron 36 tests del primer comando y 14 del TUI completo: diez escenarios y
cuatro tests del lector PTY. Se usó ejecución secuencial local, sin cambiar producto
ni CI; no hubo fallo transitorio de `openpty` ni fue necesario repetir casos.
También aprobaron guards (24), runtime `coding_workflow` (34) y `guardrails` (4),
core `tool_hook_evaluation`/`tool_hooks` (6/4), application `tool_hooks` (3) y los
dos tests focales de `output`. Estos no son totales de suites no ejecutadas.
Clippy de huginn/runtime/guards con todos los targets/features y `-D warnings`,
formato, arquitectura, Markdown e inventario Python se verifican en esta capa.

Los logs primarios están en el temporal aprobado `huginn-cli-chain-evidence/`.
El respaldo `ee44ab8` y el plan RPI conservan la evidencia anterior; no se atribuye
a esta ejecución. No se ejecutan aquí CI global, cobertura Rust del 87 %, Sonar ni
gates remotos: su validación integrada corresponde al orquestador antes del PR.
