'use strict';

const {describe, it} = require('node:test');
const assert = require('node:assert/strict');
const {spawnSync} = require('node:child_process');
const {mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync} = require('node:fs');
const {join} = require('node:path');
const {tmpdir} = require('node:os');
const {parseMetadata, parseNameStatusDiff, resolveAffected} = require('./ci-affected-crates.cjs');

const repo = '/repo';
const metadata = {
  packages: [
    {id: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', name: 'leaf', manifest_path: '/repo/crates/leaf/Cargo.toml', dependencies: []},
    {id: 'isolated 1.0.0 (path+file:///repo/crates/isolated)', name: 'isolated', manifest_path: '/repo/crates/isolated/Cargo.toml', dependencies: []},
    {id: 'external 1.0.0 (registry+https://example.invalid)', name: 'external', manifest_path: '/registry/external/Cargo.toml', dependencies: []},
    {id: 'normal-consumer 1.0.0 (path+file:///repo/crates/normal-consumer)', name: 'normal-consumer', manifest_path: '/repo/crates/normal-consumer/Cargo.toml', dependencies: [{pkg: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', kind: null}]},
    {id: 'build-consumer 1.0.0 (path+file:///repo/crates/build-consumer)', name: 'build-consumer', manifest_path: '/repo/crates/build-consumer/Cargo.toml', dependencies: [{pkg: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', kind: 'build'}]},
    {id: 'dev-consumer 1.0.0 (path+file:///repo/crates/dev-consumer)', name: 'dev-consumer', manifest_path: '/repo/crates/dev-consumer/Cargo.toml', dependencies: [{pkg: 'build-consumer 1.0.0 (path+file:///repo/crates/build-consumer)', kind: 'dev'}]},
  ],
  resolve: {nodes: [
    {id: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', deps: []},
    {id: 'isolated 1.0.0 (path+file:///repo/crates/isolated)', deps: []},
    {id: 'normal-consumer 1.0.0 (path+file:///repo/crates/normal-consumer)', deps: [{pkg: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', dep_kinds: [{kind: null}]}]},
    {id: 'build-consumer 1.0.0 (path+file:///repo/crates/build-consumer)', deps: [{pkg: 'leaf 1.0.0 (path+file:///repo/crates/leaf)', dep_kinds: [{kind: 'build'}]}]},
    {id: 'dev-consumer 1.0.0 (path+file:///repo/crates/dev-consumer)', deps: [{pkg: 'build-consumer 1.0.0 (path+file:///repo/crates/build-consumer)', dep_kinds: [{kind: 'dev'}]}]},
    {id: 'external 1.0.0 (registry+https://example.invalid)', deps: []},
  ]},
  workspace_members: [
    'leaf 1.0.0 (path+file:///repo/crates/leaf)',
    'isolated 1.0.0 (path+file:///repo/crates/isolated)',
    'normal-consumer 1.0.0 (path+file:///repo/crates/normal-consumer)',
    'build-consumer 1.0.0 (path+file:///repo/crates/build-consumer)',
    'dev-consumer 1.0.0 (path+file:///repo/crates/dev-consumer)',
  ],
};

function resolve(paths, fixture = metadata) {
  return resolveAffected(paths, fixture, repo);
}

function runScript({base, head, git, cargo} = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'ci-affected-crates-'));
  try {
    const bin = join(dir, 'bin');
    mkdirSync(bin);
    const output = join(dir, 'github-output');
    writeFileSync(output, '');
    if (git !== undefined) writeFileSync(join(bin, 'git'), `#!/bin/sh\n${git}\n`, {mode: 0o755});
    if (cargo !== undefined) writeFileSync(join(bin, 'cargo'), `#!/bin/sh\n${cargo}\n`, {mode: 0o755});
    const result = spawnSync(process.execPath, [join(__dirname, 'ci-affected-crates.cjs')], {
      cwd: dir,
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        GITHUB_OUTPUT: output,
        GITHUB_STEP_SUMMARY: join(dir, 'summary'),
        BASE: base ?? '',
        HEAD: head ?? '',
      },
    });
    assert.equal(result.status, 0, result.stderr);
    return {output: readFileSync(output, 'utf8'), stdout: result.stdout};
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}

describe('git name-status diff parser', () => {
  it('includes both paths for rename and copy records', () => {
    assert.deepEqual(parseNameStatusDiff('R100\0crates/a/src/lib.rs\0crates/b/src/lib.rs\0'), [
      'crates/a/src/lib.rs',
      'crates/b/src/lib.rs',
    ]);
    assert.deepEqual(parseNameStatusDiff('C100\0crates/a/src/lib.rs\0crates/b/src/lib.rs\0'), [
      'crates/a/src/lib.rs',
      'crates/b/src/lib.rs',
    ]);
  });

  it('falls back when a rename destination is outside the workspace', () => {
    const paths = parseNameStatusDiff('R100\0crates/leaf/src/lib.rs\0outside/lib.rs\0');
    assert.equal(resolve(paths).mode, 'workspace');
  });

  it('rejects malformed status/path records', () => {
    assert.throws(() => parseNameStatusDiff('R100\0crates/a/src/lib.rs\0'), /incomplete/);
    assert.throws(() => parseNameStatusDiff('M\0'), /incomplete/);
  });
});

describe('Bazel advisory workflow contract', () => {
  it('skips Bazel steps only when the classifier returns exactly docs-only', () => {
    const workflow = readFileSync(join(__dirname, '../workflows/bazel.yml'), 'utf8');
    const stepConditions = [...workflow.matchAll(/if: steps\.affected\.outputs\.mode != 'docs-only'/g)];
    assert.equal(stepConditions.length, 9, 'all Bazel setup/check/test steps must retain the conservative condition');
    assert.match(workflow, /BASE: \$\{\{ github\.event\.pull_request\.base\.sha \|\| github\.event\.before \}\}/);
    assert.match(workflow, /HEAD: \$\{\{ github\.event\.pull_request\.head\.sha \|\| github\.sha \}\}/);
    assert.doesNotMatch(workflow, /continue-on-error:\s*true/);
    assert.match(workflow, /name: Bazel \(non-blocking\)/);
    assert.match(workflow, /name: Bazel test \(Linux, advisory\)/);
    assert.match(workflow, /bazel mod deps --lockfile_mode=error/);
    assert.match(workflow, /python3 dev\/bazel\/generate_build_files\.py --check/);
    assert.match(workflow, /python3 dev\/bazel\/check_inventory\.py/);
    assert.match(workflow, /python3 -m unittest dev\.bazel\.test_check_inventory dev\.bazel\.test_generate_build_files/);
    assert.match(workflow, /bazel test \/\/\.\.\. --lockfile_mode=error/);
    assert.doesNotMatch(workflow, /paths-ignore:/);
  });
});

describe('affected crate shadow resolver', () => {
  it('reports documentation-only changes without selecting Rust crates', () => {
    const result = resolve(['docs/README.md', 'README.md', '.agents/rpi/plan/tasks/example.md']);
    assert.deepEqual(result, {
      mode: 'docs-only',
      packages: [],
      reason: 'documentation-only changes do not select Rust crates',
    });
  });

  it('recognizes only explicit documentation locations as docs-only', () => {
    for (const file of ['crates/leaf/README.md', 'apps/rook/dashboard/README.md', 'dev/bazel/README.md', 'docs/muninn/specs/tck-baseline.csv']) {
      assert.equal(resolve([file]).mode, 'workspace', file);
    }
    assert.equal(resolve(['.agents/agentsync.toml']).mode, 'workspace');
  });

  it('selects Rust crates when code changes are mixed with documentation', () => {
    const result = resolve(['docs/README.md', 'crates/isolated/src/lib.rs']);
    assert.equal(result.mode, 'packages');
    assert.deepEqual(result.packages, ['isolated']);
  });

  it('does not classify Markdown inside code or package directories as docs-only', () => {
    for (const file of [
      'crates/leaf/README.md',
      'apps/rook/dashboard/README.md',
      'dev/bazel/README.md',
      '.github/workflows/notes.md',
    ]) {
      assert.equal(resolve([file]).mode, 'workspace', file);
    }
  });

  it('returns only a leaf crate when no workspace package depends on it', () => {
    assert.deepEqual(resolve(['crates/isolated/src/lib.rs']).packages, ['isolated'], resolve(['crates/isolated/src/lib.rs']).reason);
  });

  it('closes over normal, build, and dev dependency edges', () => {
    assert.deepEqual(resolve(['crates/leaf/tests/ownership.rs']).packages, ['build-consumer', 'dev-consumer', 'leaf', 'normal-consumer']);
    assert.deepEqual(resolve(['crates/build-consumer/examples/demo.rs']).packages, ['build-consumer', 'dev-consumer']);
    assert.deepEqual(resolve(['crates/leaf/Cargo.toml']).packages, ['workspace']);
  });

  it('falls back for global paths, unmapped files, and ambiguous paths', () => {
    for (const file of ['Cargo.lock', 'rust-toolchain.toml', '.github/workflows/ci.yml', '.cargo/config.toml', 'Cargo.toml', '.bazelrc', 'MODULE.bazel', 'MODULE.bazel.lock', 'BUILD.bazel', 'crates/leaf/BUILD.bazel', 'dev/bazel/defs.bzl', '.github/scripts/ci-workflow.test.cjs', '.github/ci-path-filters.json', 'unknown/file.txt']) {
      const result = resolve([file]);
      assert.deepEqual(result.packages, ['workspace'], file);
      assert.equal(result.mode, 'workspace');
      assert.ok(result.reason.length > 0);
    }
    const overlapping = structuredClone(metadata);
    overlapping.workspace_members.push('overlap');
    overlapping.packages.push({id: 'overlap', name: 'overlap', manifest_path: '/repo/crates/leaf/Cargo.toml', dependencies: []});
    assert.equal(resolve(['crates/leaf/src/main.rs'], overlapping).mode, 'workspace');
  });

  it('falls back when no changed paths are supplied or a path is ambiguous', () => {
    assert.equal(resolve([]).mode, 'workspace');
    assert.equal(resolve(null).mode, 'workspace');
    assert.equal(resolve(['']).mode, 'workspace');
    assert.equal(resolve(['/repo/crates/leaf/src/lib.rs']).mode, 'workspace');
    assert.equal(resolve(['../outside.rs']).mode, 'workspace');
  });

  it('falls back for added/deleted package paths that metadata cannot map', () => {
    assert.deepEqual(resolve(['crates/new-package/src/lib.rs']).packages, ['workspace']);
    assert.deepEqual(resolve(['crates/deleted-package/Cargo.toml']).packages, ['workspace']);
  });

  it('falls back for metadata parse and graph errors', () => {
    assert.deepEqual(resolve(['crates/leaf/src/lib.rs'], '{bad json}').packages, ['workspace']);
    const invalid = structuredClone(metadata);
    invalid.resolve.nodes[0].deps = [{pkg: 'missing-package', dep_kinds: [{kind: 'normal'}]}];
    assert.deepEqual(resolve(['crates/leaf/src/lib.rs'], invalid).packages, ['workspace']);
  });

  it('validates metadata structure and retains non-src package ownership', () => {
    assert.equal(parseMetadata(metadata).members.size, 5);
    const result = resolve(['crates/leaf/build.rs']);
    assert.equal(result.mode, 'packages');
  });
});

describe('affected crate executable contract', () => {
  const validBase = 'a'.repeat(40);
  const validHead = 'b'.repeat(40);

  it('writes workspace fallback for an initial push with an invalid base', () => {
    const result = runScript({head: validHead});
    assert.equal(result.output, 'mode=workspace\n');
    assert.match(result.stdout, /\*\*Mode:\*\* workspace/);
  });

  it('writes workspace fallback for workflow_dispatch with an invalid head', () => {
    const result = runScript({base: validBase});
    assert.equal(result.output, 'mode=workspace\n');
  });

  it('classifies a synthetic package diff using the real BASE and HEAD environment contract', () => {
    const result = runScript({
      base: validBase,
      head: validHead,
      git: 'test "$1" = diff && test "$5" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb || exit 2; printf "M\\0crates/isolated/src/lib.rs\\0"',
      cargo: `printf '%s' '${JSON.stringify(metadata)}' | sed "s|/repo|$PWD|g"`,
    });
    assert.equal(result.output, 'mode=packages\n');
    assert.match(result.stdout, /\*\*Mode:\*\* packages/);
    assert.match(result.stdout, /\*\*Plan:\*\* isolated/);
  });

  it('never classifies a git diff failure as docs-only', () => {
    const result = runScript({base: validBase, head: validHead, git: 'echo failed >&2; exit 1'});
    assert.equal(result.output, 'mode=workspace\n');
    assert.doesNotMatch(result.stdout, /\*\*Mode:\*\* docs-only/);
  });

  it('never classifies cargo metadata failure as docs-only', () => {
    const result = runScript({
      base: validBase,
      head: validHead,
      git: 'printf "M\\0crates/leaf/src/lib.rs\\0"',
      cargo: 'echo metadata failed >&2; exit 1',
    });
    assert.equal(result.output, 'mode=workspace\n');
    assert.doesNotMatch(result.stdout, /\*\*Mode:\*\* docs-only/);
  });

  it('emits only exact docs-only for explicit documentation-only paths', () => {
    const result = runScript({
      base: validBase,
      head: validHead,
      git: 'printf "M\\0docs/README.md\\0"',
    });
    assert.equal(result.output, 'mode=docs-only\n');
    assert.match(result.stdout, /\*\*Mode:\*\* docs-only/);
  });

  it('does not emit docs-only for non-documentation or malformed diff output', () => {
    for (const git of [
      'printf "M\\0crates/leaf/src/lib.rs\\0"',
      'printf "M\\0docs/README.md"',
    ]) {
      const result = runScript({base: validBase, head: validHead, git, cargo: 'exit 1'});
      assert.equal(result.output, 'mode=workspace\n');
      assert.doesNotMatch(result.stdout, /\*\*Mode:\*\* docs-only/);
    }
  });
});
