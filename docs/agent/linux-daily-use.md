# Agent Linux daily-use checklist

This checklist records a real coding session against a tool-capable Chat
Completions endpoint on a Linux host. It is the manual acceptance step
referenced from `validation.md`. Each step has a fixed command and a one-line
note you paste into the daily record.

Target: **10–15 minutes** of focused use per session, with at least one session
recorded per day across several days. The point is to expose rough edges that
deterministic fixtures cannot, not to validate reasoning quality.

## Prerequisites

- Linux host, user account, network access to the configured endpoint.
- Rust 1.89 toolchain (`rustup install 1.89` or the workspace's toolchain file).
- A clean checkout of this repository.
- Environment: `AGENT_BASE_URL` (must end in `/v1`), `AGENT_MODEL`,
  `AGENT_API_KEY`. Set `api_key_env = ""` in the config for a local endpoint
  without authentication.

## Before the session

```bash
# 1. Smoke — verifies the install on this Linux host. Does not contact a model.
./scripts/agent-linux-smoke.sh
```

If the smoke fails, fix the local install before continuing. The record below
only counts sessions that started on a green smoke.

## Workflow — copy, paste, annotate

Run each step in order, in a real repository of your choice (any small project
works). Replace the example workspace with an absolute path.

```bash
WS="/absolute/path/to/disposable-repository"
agent --workspace "$WS" doctor
```

Set `AGENT_BASE_URL`, `AGENT_MODEL` and the credential environment variable in
the invoking shell before starting. Keep credentials out of the acceptance record.
`doctor` checks local configuration; it does not establish endpoint compatibility.

**Note:** _doctor result here (services green / red, what was missing)._

```bash
agent --workspace "$WS" run "List the files in src/ and summarise their purpose."
```

**Note:** _response quality, latency to first token, any tool errors._

```bash
# Start an interactive session for edits and commands requiring approval.
agent --workspace "$WS" chat
# Prompt: In README.md, fix the first typo you find.
```

Use a disposable repository or branch with a known typo. Approve the displayed
diff when prompted. `run` denies effects without explicit `--allow` grants;
use `chat` here to exercise individual approval prompts.

**Note:** _diff rendered correctly? Approval
flow clear? File written atomically?_

```bash
# In the same chat, request a test appropriate to this repository.
# For a Rust project: Run 'cargo fmt --check' and report the result.
```

**Note:** _approval asked before execution? Output truncated sensibly? Exit
code returned to the model? Ctrl+C cancelled the process group?_

```bash
# In the same chat, give it another prompt and press Ctrl+C mid-turn.
# Then enter /quit to leave the chat and restart the process for the resume check.
```

**Note:** _did the session save cleanly? Was the in-flight tool call marked as
unknown completion rather than auto-replayed?_

```bash
# Resume — confirm the interrupted state is visible, not silently re-run.
agent --workspace "$WS" sessions
agent --workspace "$WS" resume <paste-uuid-here>
```

**Note:** _did resume show the unfinished work? Did the next turn require a
fresh approval for any effectful action?_

```bash
# Context — drive a longer conversation that should trigger compaction.
agent --workspace "$WS" chat
# Continue with substantive prompts and varied tool calls until compaction occurs.
# Use --json before chat to observe the 'compacted' event.
```

Prompt count alone does not guarantee compaction. If the default context budget
is too large for a short acceptance run, use a separate TOML configuration with
a smaller valid `context_tokens` budget and `max_output_tokens` below half that
budget. Record those values and whether compaction actually occurred.

**Note:** _did compaction fire? Did the original history stay intact (re-run
`agent sessions` and inspect, or query the SQLite record directly)? If
compaction failed, did the turn stop without losing the originals?_

## After the session

Append a one-line entry per session to `docs/agent/validation.md` under the
"Linux daily-use record" heading, in this exact shape:

```
- YYYY-MM-DD host=<hostname> rustc=<version> endpoint=<provider/model>
  steps=<which steps completed 1–7> result=<ok|degraded|failed>
  notes=<one-line summary or "see issue #N">
```

Then commit both this checklist's *edits* and the validation update in the
same change so the record travels with the code.

## What "degraded" and "failed" mean

- **ok** — every completed step behaved as documented. UX rough edges are
  fine; correctness regressions are not.
- **degraded** — at least one step completed but with a workaround (e.g.
  approval wording unclear, diff truncated but visible, output truncated but
  sufficient).
- **failed** — at least one step did not complete; record the symptom and the
  closest log line.

Five consecutive `ok` sessions across at least two different repositories are
the bar for closing this item in `validation.md`. Anything short of that keeps
the item open.
