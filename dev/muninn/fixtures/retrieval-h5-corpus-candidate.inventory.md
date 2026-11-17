# H5 corpus candidate source inventory

## Review status

This inventory supports a review-only draft. It does not establish relevance, adequacy, representativeness, owner approval, freeze, or authorization to measure. The query and candidate text, source assignments, and hashes require owner review.

## Baseline and scope

- Baseline commit: `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`.
- Inventory scope: tracked Markdown files directly under `docs/muninn/` at the baseline; 29 files.
- Eligible allowlist: 21 files listed below. The source-byte hashes in the companion provenance JSON are read from baseline Git objects, not the current worktree.
- Excluded: modified pre-existing `docs/muninn/retrieval-h5-runner.md`; all H5 pilot documents and results (`retrieval-h5-*` and `retrieval-judgment-pilot-v1.md`); C0/C1 source/fixture references (`export-minilm-c0-reference.md`, `c1-execution-runner.md`). This also excludes pilot corpus, labels, hypothetical labels, generated evidence, and untracked material.

## Eligible source allowlist

- `docs/muninn/NOTICE.md`
- `docs/muninn/README.md`
- `docs/muninn/context.md`
- `docs/muninn/cross-memory.md`
- `docs/muninn/enrichment.md`
- `docs/muninn/entity-resolution.md`
- `docs/muninn/extraction.md`
- `docs/muninn/gm01-acceptance.md`
- `docs/muninn/gm01-deviations.md`
- `docs/muninn/gm01-runner.md`
- `docs/muninn/h3-execution-memory.md`
- `docs/muninn/h4-measurement.md`
- `docs/muninn/long-term-isolation-deletion.md`
- `docs/muninn/minilm-product.md`
- `docs/muninn/owner-session-namespace.md`
- `docs/muninn/preference-filters.md`
- `docs/muninn/reference-api-artifacts.md`
- `docs/muninn/reference-lock.md`
- `docs/muninn/reference-mcp.md`
- `docs/muninn/reference-tck.md`
- `docs/muninn/schema-operations.md`

## Candidate source assignment

The corpus candidate contains one independently worded query and ten neutral, unlabelled candidate summaries. The query maps to `docs/muninn/context.md`; the ten candidate IDs map one-to-one to the ten source paths recorded in `retrieval-h5-corpus-candidate.provenance.json`. No query/candidate judgments, scores, ranks, labels, or retrieval outputs are included. The source mapping is a draft for owner review, not an assertion that any candidate is relevant.

## License and sensitivity

The baseline repository root contains an MIT license. This records license context only; it does not establish owner or third-party rights approval for these excerpts. No project sensitivity classification is present. The selected candidate summaries were manually inspected for credentials and sensitive payloads and none were identified. This check does not establish a sensitivity classification. Owner review of all source paths and candidate text remains required.
