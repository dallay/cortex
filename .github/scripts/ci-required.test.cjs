const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { evaluateRequiredChecks } = require('./ci-required.cjs');

const workflow = fs.readFileSync('.github/workflows/ci.yml', 'utf8');

const base = {
  eventName: 'pull_request',
  actor: 'contributor',
  fork: false,
  changes: { backend: 'false', frontend: 'false', docker: 'false', github: 'false', docs: 'false', labeler: 'false' },
  results: {
    changes: 'success', 'test-ci-policy': 'skipped', 'build-dashboard': 'skipped', fmt: 'skipped', markdown: 'skipped',
    'test-labeler': 'skipped', clippy: 'skipped', check: 'skipped', test: 'skipped',
    'test-frontend': 'skipped', 'test-e2e': 'skipped', doc: 'skipped', audit: 'skipped',
    'trivy-fs': 'success', 'gitleaks-pr': 'success', 'semgrep-pr': 'skipped', sonar: 'skipped',
  },
};

function evaluate(overrides = {}) {
  const input = { ...structuredClone(base), sonarTokenPresent: true };
  Object.assign(input, overrides);
  input.changes = { ...base.changes, ...(overrides.changes || {}) };
  input.results = { ...base.results, ...(overrides.results || {}) };
  return evaluateRequiredChecks(input);
}

test('required aggregate checks out the repository before running its validator', () => {
  const job = workflow.split('  required:\n')[1]?.split(/\n  [a-zA-Z0-9_-]+:\n/)[0];
  assert.ok(job, 'required job exists');
  assert.match(job, /uses: actions\/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10/);
  assert.match(job, /run: node \.github\/scripts\/ci-required-run\.cjs/);
  assert.ok(job.indexOf('actions/checkout') < job.indexOf('ci-required-run.cjs'));
});

test('frontend path filter only matches dashboard paths, including dashboard E2E', () => {
  const frontendFilter = workflow.match(/            frontend:\n((?:              - .*\n)+)/)?.[1];
  assert.ok(frontendFilter, 'frontend filter exists');
  const patterns = [...frontendFilter.matchAll(/^\s+- ['\"]?([^'\"\n]+)['\"]?$/gm)].map((match) => match[1]);
  assert.deepEqual(patterns, ['apps/rook/dashboard/**']);
  const matches = (file) => patterns.some((pattern) => file.startsWith(pattern.slice(0, -2)));
  assert.equal(matches('apps/rook/dashboard/src/main.ts'), true);
  assert.equal(matches('apps/rook/dashboard/e2e/login.spec.ts'), true);
  assert.equal(matches('.github/workflows/ci.yml'), false);
  assert.equal(matches('.github/scripts/ci-required.cjs'), false);
  assert.equal(matches('docs/branch-protection-ruleset.md'), false);
});

test('documentation-only accepts skipped unrelated jobs and requires markdown + always-on secret scan', () => {
  const result = evaluate({
    changes: { docs: 'true' },
    results: { markdown: 'success' },
  });
  assert.equal(result.ok, true, result.errors.join('\n'));
});

test('E2E infrastructure-only path requires E2E and Trivy, while preserving unrelated skips', () => {
  const result = evaluate({
    changes: { docker: 'true' },
    results: { 'test-e2e': 'success', 'trivy-fs': 'success', 'semgrep-pr': 'success' },
  });
  assert.equal(result.ok, true, result.errors.join('\\n'));
});

test('frontend-only requires frontend tests, E2E, dashboard build and Sonar', () => {
  const result = evaluate({
    changes: { frontend: 'true' },
    results: { 'build-dashboard': 'success', 'test-frontend': 'success', 'test-e2e': 'success', sonar: 'success' },
  });
  assert.equal(result.ok, true, result.errors.join('\n'));
});

test('backend-only does not require frontend tests for Sonar eligibility', () => {
  const result = evaluate({
    changes: { backend: 'true' },
    results: { 'build-dashboard': 'success', fmt: 'success', clippy: 'success', check: 'success', test: 'success',
      'test-e2e': 'success', doc: 'success', audit: 'success', 'trivy-fs': 'success', 'semgrep-pr': 'success', sonar: 'success' },
  });
  assert.equal(result.ok, true, result.errors.join('\n'));
});

test('workflow changes require workflow-sensitive checks and reject unexpected skips', () => {
  const result = evaluate({
    changes: { github: 'true' },
    results: { markdown: 'success', 'test-labeler': 'success', 'build-dashboard': 'success', fmt: 'skipped' },
  });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\n'), /fmt.*expected success.*got skipped/i);
});

test('workflow-only changes pass when all workflow-sensitive jobs succeed', () => {
  const result = evaluate({
    changes: { github: 'true' },
    results: { 'test-ci-policy': 'success', 'build-dashboard': 'success', fmt: 'success', markdown: 'success',
      'test-labeler': 'success', clippy: 'success', check: 'success', test: 'success', 'test-frontend': 'success',
      'test-e2e': 'success', doc: 'success', audit: 'success', 'trivy-fs': 'success', 'semgrep-pr': 'success', sonar: 'success' },
  });
  assert.equal(result.ok, true, result.errors.join('\\n'));
});

test('required failing security check blocks the aggregate', () => {
  const result = evaluate({ changes: { backend: 'true' }, results: { 'trivy-fs': 'failure' } });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\\n'), /trivy-fs.*failure/i);
});

test('required failing E2E blocks the aggregate', () => {
  const result = evaluate({
    changes: { frontend: 'true' },
    results: { 'build-dashboard': 'success', 'test-frontend': 'success', 'test-e2e': 'failure', sonar: 'success' },
  });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\n'), /test-e2e.*failure/i);
});

test('Sonar analysis failure is blocking even for a fork or Dependabot PR', () => {
  const result = evaluate({
    fork: true,
    changes: { frontend: 'true' },
    results: { 'build-dashboard': 'success', 'test-frontend': 'success', 'test-e2e': 'success', sonar: 'failure' },
    sonarTokenPresent: false,
  });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\\n'), /sonar.*failure/i);
});

test('internal analysis required without token fails explicitly', () => {
  const result = evaluate({ changes: { backend: 'true' }, sonarTokenPresent: false });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\n'), /sonar.*token/i);
});

test('fork and Dependabot PRs allow explicit no-secret Sonar exception', () => {
  const fork = evaluate({
    fork: true,
    changes: { backend: 'true' },
    results: { 'build-dashboard': 'success', fmt: 'success', clippy: 'success', check: 'success', test: 'success',
      'test-e2e': 'success', doc: 'success', audit: 'success', 'trivy-fs': 'success', 'semgrep-pr': 'success', sonar: 'success' },
    sonarTokenPresent: false,
  });
  assert.equal(fork.ok, true, fork.errors.join('\n'));
  const dependabot = evaluate({ actor: 'dependabot[bot]', changes: { frontend: 'true' },
    results: { 'build-dashboard': 'success', 'test-frontend': 'success', 'test-e2e': 'success', sonar: 'success' },
    sonarTokenPresent: false,
  });
  assert.equal(dependabot.ok, true, dependabot.errors.join('\n'));
});

test('always-on Gitleaks and change detection cannot be skipped', () => {
  const result = evaluate({ results: { changes: 'skipped', 'gitleaks-pr': 'skipped' } });
  assert.equal(result.ok, false);
  assert.match(result.errors.join('\n'), /changes.*success/i);
  assert.match(result.errors.join('\n'), /gitleaks-pr.*success/i);
});
