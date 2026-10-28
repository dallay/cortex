'use strict';

const { describe, it } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const justfile = fs.readFileSync(path.join(__dirname, '../../justfile'), 'utf8');
const workflow = fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'), 'utf8');

function recipeBlock(name) {
  const start = justfile.match(new RegExp(`^${name}(?:\\s[^\\n]*)?:\\n`, 'm'));
  assert.ok(start, `just recipe ${name} exists`);
  const contentStart = start.index + start[0].length;
  const rest = justfile.slice(contentStart);
  const next = rest.search(/^[\w-]+(?:\s[^\n]*)?:/m);
  return rest.slice(0, next === -1 ? undefined : next);
}

function assertContainsAll(source, values, context) {
  for (const value of values) {
    assert.ok(source.includes(value), `${context} includes ${value}`);
  }
}

describe('local CI contract', () => {
  it('provides just ci and keeps ci-local as a compatible alias', () => {
    assert.ok(/^ci:\s*$/m.test(justfile), 'just ci recipe exists');
    assert.match(recipeBlock('ci'), /just ci-local/);
    assert.match(recipeBlock('ci-local'), /just ci-contract/);
  });

  it('runs contract, doctor and issue-labeler tests from the GitHub workflow', () => {
    const local = recipeBlock('ci-local');
    assert.match(local, /ci-contract/);
    assert.match(local, /doctor-test/);
    assert.match(local, /ci-labeler-test/);
    assert.match(workflow, /node --test \.github\/scripts\/ci-\*\.test\.cjs/);
    assert.match(recipeBlock('ci-labeler-test'), /node --test \.github\/scripts\/issue-labeler\.test\.cjs/);
    assert.match(recipeBlock('ci-install'), /pnpm install --frozen-lockfile/);
  });

  it('covers dashboard build, both coverage gates, E2E and Rust docs', () => {
    const local = recipeBlock('ci-local');
    assertContainsAll(local, ['ci-dashboard-build', 'ci-coverage-rust', 'ci-coverage-frontend', 'ci-e2e', 'ci-doc'], 'ci-local');
    assert.match(workflow, /--fail-under-lines 87/);
    assert.match(workflow, /vitest run --coverage/);
  });

  it('runs blocking local security gates and does not hide audit failures', () => {
    const local = recipeBlock('ci-local');
    for (const token of ['ci-audit', 'ci-trivy', 'ci-gitleaks', 'ci-semgrep']) {
      assert.match(local, new RegExp(token));
    }
    assert.doesNotMatch(recipeBlock('ci-audit'), /\|\|\s*true/);
    assert.match(recipeBlock('ci-audit'), /cargo audit(?:\s|$)/);
    assert.match(recipeBlock('ci-semgrep'), /semgrep==1\.126\.0/);
    assert.match(workflow, /severity: HIGH,CRITICAL/);
    assert.match(workflow, /--severity ERROR/);
    assert.match(recipeBlock('ci-semgrep'), /jq -e/);
    assert.match(recipeBlock('ci-semgrep'), /CORTEX_CI_TMPDIR/);
    assert.match(recipeBlock('ci-trivy'), /Trivy is required/);
    assert.match(workflow, /--exit-code 1/);
  });

  it('documents SonarCloud and push-only builds as non-local or separate checks', () => {
    const local = recipeBlock('ci-local');
    assert.match(local, /SonarCloud/);
    assert.match(workflow, /Cross-compile validation \(only on main\/develop push, not PRs\)/);
    assert.match(workflow, /Windows validation/);
    assert.match(recipeBlock('ci-gitleaks'), /CI_BASE_REF/);
    assert.match(recipeBlock('ci-gitleaks'), /refusing an incomplete Gitleaks range scan/);
  });
});
