# Finite MiniLM artifact operator

The `prepare_minilm` example composes the pinned manifest, bounded HTTPS fetcher
and verified artifact staging. It downloads public immutable artifacts only. It
does not run a model, expose a service or establish numerical/conformance parity.

## Private root and one-shot claim

The operator must explicitly provide an existing physical absolute directory,
owned by the process UID with mode 0700 and an accepted native ACL/filesystem
policy. The program never creates the root, broadens permissions, clears ACLs
or deletes existing data to make preparation succeed.

Preparation requires an empty root, inspected through its descriptor. An
exclusive `.muninn-prepare-once` marker permanently consumes that root for one
cooperating run. Competing callers cannot both claim it. A sync failure can leave
the marker visible with `OutcomeUnknown`; either way this invocation stops before
downloads. No automatic retry, marker deletion or switch to a fresh root occurs.

This protocol bounds this finite workflow, not every possible staging caller or
every root on the host. General quotas, orphan maintenance and crash recovery
remain separate service-exposure gates. File/directory sync requests do not prove
universal power-loss survival.

## Commands

After the operator provisions and validates the intended new private root:

```sh
cargo run -p muninn-bridge-spike --example prepare_minilm --locked --offline -- \
  prepare /physical/absolute/private-root
```

`--offline` applies to Cargo package resolution, not the example's authorized
HTTPS artifact requests. The example performs one sequential pass over the eleven
manifest files with no transport retry, then publishes using no-replace semantics.

Events distinguish successful file staging from final publication/readback:

- `artifact_staged`: verified input was written, read back and synchronized.
- `cache_artifact_verified`: a newly verified buffer was read from the accepted
  published cache; its actual byte count and digest are recorded.
- `bundle_verified`: all required artifacts passed fresh verification; the bundle
  ID matches the exact canonical receipt. `inference_executed` remains false.

Output contains public artifact paths, sizes, hashes, revision and bundle ID—not
URLs, signed queries, certificate material, credentials or native source errors.
Prepare-mode readback retains the original parent descriptor rather than resolving
the root pathname again.

A separate process can verify a known bundle without claiming, downloading,
staging or changing any permissions/data:

```sh
cargo run -p muninn-bridge-spike --example prepare_minilm --locked --offline -- \
  verify /physical/absolute/private-root <64-character-bundle-id>
```

That command validates the currently named root, receipt and files. It does not
claim a continuous inode lease across processes. A failed or consumed preparation
root is never resumed implicitly; preserve it and inspect recorded evidence
instead of removing its marker.

The explicitly reviewed three-file failed-run profile has a separate finite
`recover-known` command. It validates the exact known snapshot, creates a distinct
one-time recovery claim and copies verified prefix bytes into one new exclusive
stage without altering the original. It is not a retry of `prepare`; see
[RECOVER-KNOWN.md](RECOVER-KNOWN.md) for the stopped-worker precondition, identity
arguments, logical byte bound and remaining limits.

## Verification scope

Guard tests cover empty/nonempty roots, repeat and competing claims, pathname
replacement and both synchronization failure points. Runner tests cover argument
boundaries, refusal before preparation and mutation-free verification of a missing
bundle. Linux test harnesses are included in the development bridge image.

Live acquisition and separate read-only verification require their own recorded
outcomes. Passing unit tests does not mean model weights were downloaded, Candle
can load them, or C0/C1 and §12.3 have passed.
