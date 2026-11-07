# GitHub Quality Ruleset: Preparation and Activation

This is a maintainer procedure for the external GitHub quality requirements. Local implementation and activation on GitHub are distinct operations. This procedure does not prove that the updated workflow is published on `main` or that the external requirements are active. Activation remains pending until an administrator runs and verifies the authorized change.

Run the commands from the repository root. Read the full procedure before requesting or using activation credentials.

The `.github/scripts/ci-quality-ruleset.cjs` helper is scoped to `dallay/cortex`, default branch `main`, ruleset `17001295`, source `Repository`, active, and of type `branch`. Before preparing the payload, it compares the full editable policy against `.github/ci-quality-policy.json`: the five rules, the twelve original checks with their Apps and parameters, the conditions, the bypass actors, the name, and the strict policy. It rejects any missing, altered, duplicated, or unexpected protection. The array comparison preserves order and multiplicity; it also does not accept reorderings of the baseline. It does not pin response metadata such as `updated_at` or `node_id`. It preserves all of that policy and allows adding only these identities:

- `CodeQL`: `integration_id=57789`, App `github-advanced-security`.
- `SonarCloud Code Analysis`: `integration_id=12526`, App `sonarqubecloud`.

It does not add a `code_scanning` rule or modify severity thresholds. The current CodeQL threshold has not been verified here. A name already bound to another App, without a defined App, or duplicated blocks the operation; it is not silently replaced. Repeating the preparation does not duplicate checks.

## Validate the reviewed local copy

From a reviewed and trusted local copy, validate first without credentials:

```bash
node --test .github/scripts/ci-quality-ruleset.test.cjs
node --test .github/scripts/ci-*.test.cjs
actionlint .github/workflows/ci.yml
```

## Read-only dry run

To query the ruleset, authenticate `gh` against `github.com` with read permissions. The default command only performs a GET and shows the payload allowed by the API and the list of added requirements. It does not activate changes:

```bash
node .github/scripts/ci-quality-ruleset.cjs
# Explicit equivalent:
node .github/scripts/ci-quality-ruleset.cjs --dry-run
```

Before activating, publish and review the workflow and all its associated CI files on `main`, confirm the SonarCloud configuration, and observe both external checks on a real PR. The scan must run on every PR, including documentation-only ones, without `sonar_exempt`, with `-Dsonar.qualitygate.wait=true` and `-Dsonar.qualitygate.timeout=300`. Analyzing documentation PRs adds runner, coverage, and analysis consumption; waiting for the quality gate can also extend the job up to its timeout. Forks and Dependabot without credentials remain blocked: do not use `pull_request_target` or automatically transfer fork code into internal branches to obtain secrets.

## Activation

**Warning:** the following command modifies GitHub. Use it only with activation authorization, administrative permissions, and with no other administrator editing the ruleset. It is not part of local validation:

```bash
node .github/scripts/ci-quality-ruleset.cjs --apply
```

The preflight resolves `main` once to an immutable commit SHA. It compares the SHA-256 of the content of these nine files at that same commit against the reviewed local copy; a missing or different file blocks the PUT:

- `.github/workflows/ci.yml`
- `.github/ci-path-filters.json`
- `.github/scripts/ci-required.cjs`
- `.github/scripts/ci-required-run.cjs`
- `.github/scripts/ci-required.test.cjs`
- `.github/scripts/ci-workflow.test.cjs`
- `.github/scripts/ci-quality-ruleset.cjs`
- `.github/scripts/ci-quality-ruleset.test.cjs`
- `.github/ci-quality-policy.json`

Publishing only `ci.yml` is not enough. Documentation files are not execution requirements. The dry-run remains available even if `main` still has the previous CI; it does not imply that `--apply` can pass the publication check.

The helper also checks execution markers across the whole PR and waits for the quality gate. This is a conservative check of the YAML format in blocks used here, not a YAML interpreter or a workflow execution test; unsupported formats or conditions require review, not a bypass. The helper reads remote contents as data; it never executes remote or PR code. It runs `gh` with separate arguments, without a shell.

Before the PUT, the helper saves the original snapshot with restricted permissions under `opencode/ci-quality-ruleset-*` in the system temporary directory and prints its path. Before writing, it resolves `main` again and aborts if its SHA changed, then rereads the ruleset and blocks detected policy changes. This is not transactional: the API does not provide an atomic compare-and-write operation, and `main` can still advance between the final ref check and the PUT. Avoid concurrent publications and administrative edits during activation. After the PUT, the helper reads the ruleset again and compares all preserved fields and both new identities. A write or verification error requires manual inspection with the snapshot; it does not perform an automatic rollback that could erase another administrator's changes.
