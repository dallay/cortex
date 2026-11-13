# C1 execution runner with provenance

`dev/muninn/c1_runner.py` runs external stages (exporters) against the exact
bytes of frozen inputs, captures output and logs, verifies what the process
claims to have consumed, and writes an attempt receipt only when all checks pass.

The module does not run inference, load models, or access Neo4j. It only
orchestrates external processes and verifies their results.

## Relationship with admission

`dev/muninn/c1_execution.py` handles admission and byte snapshots. The runner
uses it as its foundation: it admits the frozen manifest using the digest the
operator obtains from an independently reviewed record, and reads each input
only once through `read_frozen_input`.

| Phase | `c1_execution` | `c1_runner` |
| --- | --- | --- |
| Manifest | `admit` authenticates bytes and contract | executes and verifies again |
| Inputs | `read_frozen_input` returns bytes and digest | delivers them to the process and revalidates them |
| Outputs | has no knowledge of them | captures, seals, and rereads them |

## Running a stage

A stage (`Stage`) declares:

- `argv`: command and arguments. `{rol}` is replaced by the path of the file
  prepared in the private area, `{output_dir}` by the stage output directory,
  and `{attempt}` by the attempt directory.
- `build`: trusted build record (`BuildBinding`) with the expected executable
  digest, the expected digest of each source, and the toolchain.
- `inputs`: role, relative path in the manifest, and delivery method (`stdin`
  or `path`). Only one input per stage can use `stdin`.
- `outputs`: filenames the stage must produce inside its output directory.
- `consumed`: checks requiring a JSON output field (or captured `stdout`) to
  contain the digest of the consumed snapshot.

The current exporters (`export_minilm_c1.py` and the equivalent Rust binary)
read a file path and write a single result to `stdout`. The runner can bind
`consumed_input_sha256` to that output without changing the exporters.

## Attempt receipt

`run_attempt` creates a new attempt directory and rejects an existing one.
Each stage runs without a shell, with closed `stdin` when unused, a temporary
working directory, and an explicit environment without inherited credentials.
Before and after execution, the runner verifies the build identity.

Only a successful attempt produces `attempt-receipt.json`. The receipt contains:

- the SHA-256 of the admitted frozen manifest;
- build identity for each stage;
- roles, relative paths, lengths, and SHA-256 values of inputs;
- paths, lengths, and SHA-256 values of outputs and captured logs;
- final status `EXECUTION_RECORDED`.

`EXECUTION_RECORDED` does not mean `PASS`, C1 acceptance, or H1 closure. It only
means that the declared execution and provenance passed the runner checks.

## Minimum consumption probe

The regression runs a Python process that reads the supplied input path and
returns the SHA-256 of those bytes. With the correct digest,
`attempt-receipt.json` exists. With a false digest, the runner raises `Blocked`
and leaves the attempt without a receipt. A nonzero exit also leaves no receipt.
