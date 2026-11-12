# OpenCode + Muninn memory benchmark runner

An automated, disposable harness that measures whether an OpenCode agent can
retrieve stored Muninn preferences and messages through the Muninn MCP server.

The harness answers one question per case: *given a fixed synthetic fixture,
does the agent retrieve the right record and answer with it?* It reports what
happened in that scenario. It does not score memory quality in general, it
does not compare Muninn with another memory system, and it does not claim
global parity.

## Boundaries

The runner is a development tool. It must never be pointed at production or
personal data.

* Every run uses a disposable Neo4j container with an ephemeral password.
* No container publishes a port, mounts the host home directory, or mounts the
  Docker socket.
* The runner container joins an internal network (for Neo4j) and an egress
  network (for the model provider). Neo4j joins the internal network only.
* The provider credential is imported by the operator into a Docker volume.
  The harness never reads it and never writes it to the evidence.
* The agent runs with least privilege: every tool is denied, and only the
  `muninn_*` MCP tools are allowed.
* The harness never runs an inference without `--authorize-inference`.

## Layout

| Path | Purpose |
| --- | --- |
| `membench/scenario.py` | Scenario schema, validation, owner scopes |
| `membench/compose.py` | Container topology, isolation rules, Compose rendering |
| `membench/config.py` | Rendered OpenCode configuration and agent definition |
| `membench/events.py` | Parser for the OpenCode JSON event stream |
| `membench/metrics.py` | Assertions, verdicts, cross-mode diagnosis |
| `membench/mcpclient.py` | Minimal MCP client over stdio |
| `membench/preflight.py` | Environment checks that run no inference |
| `membench/runner.py` | Orchestration of one run |
| `membench/sanitize.py` | Redaction of stored evidence |
| `scenarios/synthetic-memory-recall-v2.json` | The fixture and the expectations |
| `bin/seed.sh` | Seeds the disposable database from inside the container |
| `Dockerfile.runner` | Runner image: pinned OpenCode + Muninn |

## Evaluation protocol

### Modes

| Mode | What it isolates |
| --- | --- |
| A | The retrieval contract itself: the harness queries the MCP server directly |
| B | Tool use: the agent is told which tool to call |
| C | The agent decision: the agent decides whether to call a tool |
| D | The model: the answer is given in the prompt and no tool is available |

### Conditions

| Condition | Memory state |
| --- | --- |
| C0 | The owner scope of the case is empty |
| C1 | The owner scope of the case holds the seeded records |
| C2 | Oracle: the stored information is part of the prompt |

Every case owns a distinct owner scope, so cases never observe each other.
The empty scope of a case holds no record at all.

### Diagnosis

The harness locates the first broken link:

| Observation | Diagnosis |
| --- | --- |
| A fails | `RETRIEVAL_FAILURE` |
| A passes, B fails | `TOOL_USE_FAILURE` |
| B passes, C fails | `AUTONOMY_FAILURE` |
| C fails, D passes | `MODEL_UTILIZATION_FAILURE` |
| D fails | `MODEL_ANSWER_FAILURE` |

### Cases

| Case | Question |
| --- | --- |
| `category-exact` | The exact category returns the preference |
| `category-equivalent` | Equivalent wording finds the preference (needs a semantic embedder) |
| `category-incompatible` | An unrelated preference is not presented as the answer |
| `preference-updated` | The current version is returned and the superseded one is not |
| `preference-missing` | The agent reports that nothing is stored |
| `other-owner` | Records of another owner scope stay invisible |
| `message-identifier` | The message retrieval path returns the stored identifier |

## Isolation model

| Item | Isolation |
| --- | --- |
| Database | New Neo4j container per repetition |
| Fixture | Seeded per repetition from the scenario file |
| Agent state | Fresh `HOME` and `OPENCODE_DB` per run, on tmpfs |
| Configuration | Rendered per run and mounted read-only |
| Credentials | Docker volume, imported by the operator, never copied into the run |
| Provider | The model identifier is given per run; the harness stops if the catalog does not list it |

## Running it

Build the runner image from the repository root:

```sh
docker build -f dev/muninn/opencode/Dockerfile.runner -t membench-opencode-runner:2.2.7 .
```

Build it with the local MiniLM embedder when the semantic case must run:

```sh
docker build --build-arg MUNINN_FEATURES=minilm-cpu \
  -f dev/muninn/opencode/Dockerfile.runner -t membench-opencode-runner:2.2.7 .
```

Import the provider credential once (the file is an OpenCode credential export):

```sh
PYTHONPATH=dev/muninn/opencode python3 -m membench.cli auth import --file ~/auth.json
```

Check the environment without running an inference:

```sh
PYTHONPATH=dev/muninn/opencode python3 -m membench.cli preflight \
  --model provider/model \
  --scenario dev/muninn/opencode/scenarios/synthetic-memory-recall-v2.json
```

Show the plan and the number of inferences it requires:

```sh
PYTHONPATH=dev/muninn/opencode python3 -m membench.cli plan \
  --scenario dev/muninn/opencode/scenarios/synthetic-memory-recall-v2.json --repetitions 5
```

Execute the run:

```sh
PYTHONPATH=dev/muninn/opencode python3 -m membench.cli run \
  --model provider/model \
  --scenario dev/muninn/opencode/scenarios/synthetic-memory-recall-v2.json \
  --repetitions 5 \
  --embedding-provider minilm \
  --embedding-bundle-path /path/to/minilm-artifacts/<bundle-id> \
  --embedding-bundle-id <bundle-id> \
  --authorize-inference
```

Unit tests (no inference, no database, no Docker):

```sh
just ci-muninn-opencode
```

## Evidence

Every run writes to `.agents/rpi/plan/tasks/opencode-docker-muninn-evaluation/evidence/<run-id>/`:

| File | Content |
| --- | --- |
| `run-manifest.json` | Scenario, model, images, preflight states, verdicts, diagnosis |
| `report.md` | Human readable report |
| `runs/*-events.json` | Sanitized event stream of each agent run |

The stored evidence contains no credentials, no tokens, and no embedding
vectors. The report states the scope of the result in every run.

## Honest states

The preflight reports what it actually proved:

| State | Meaning |
| --- | --- |
| `PASS` | The check succeeded |
| `CATALOG_VERIFIED` | The model identifier is listed; quota and availability are not proven |
| `AUTH_CONFIGURED` | A credential exists; a real request is not proven |
| `NOT_TESTED` | The check did not run, for example because it would cost an inference |
| `FAIL` | The check failed and the run stops |

A model that appears in the catalog is not a model that will answer. The
harness never switches to another model, and it never reports a run that did
not execute.

## What the harness does not prove

* It does not prove that Muninn has the same quality as another memory system.
* It does not prove that a model in general can use Muninn; it measures one
  model, one scenario, and one fixture.
* It does not cover long-horizon memory, conflict resolution across sessions,
  or the short-term store.
* It does not prove that the prompt wording is optimal; the fixture is
  synthetic and versioned.
