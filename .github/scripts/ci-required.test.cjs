'use strict';

const { describe, it } = require('node:test');
const assert = require('node:assert/strict');
const { evaluateRequiredChecks } = require('./ci-required.cjs');

const successResults = {
  changes: 'success',
  'build-dashboard': 'success',
  fmt: 'success',
  markdown: 'success',
  'test-labeler': 'success',
  clippy: 'success',
  check: 'success',
  test: 'success',
  'test-frontend': 'success',
  'test-e2e': 'success',
  doc: 'success',
  audit: 'success',
  'trivy-fs': 'success',
  'gitleaks-pr': 'success',
  'semgrep-pr': 'success',
  sonar: 'success',
  coverage: 'success',
  'coverage-frontend': 'success',
};

function input(changes, results = successResults) {
  return {
    eventName: 'pull_request',
    actor: 'developer',
    fork: false,
    sonarTokenPresent: true,
    changes,
    results,
  };
}

describe('required coverage checks', () => {
  it('requires Rust coverage for backend changes', () => {
    const result = evaluateRequiredChecks(input({ backend: 'true' }));
    assert.equal(result.ok, true);
  });

  it('fails the gate when applicable Rust coverage fails', () => {
    const result = evaluateRequiredChecks(input({ backend: 'true' }, { ...successResults, coverage: 'failure' }));
    assert.equal(result.ok, false);
    assert.ok(result.errors.includes('coverage: expected success, got failure'));
  });

  it('requires frontend coverage for frontend changes', () => {
    const result = evaluateRequiredChecks(input({ frontend: 'true' }));
    assert.equal(result.ok, true);
  });

  it('fails the gate when applicable frontend coverage is missing', () => {
    const result = evaluateRequiredChecks(input({ frontend: 'true' }, { ...successResults, 'coverage-frontend': 'skipped' }));
    assert.equal(result.ok, false);
    assert.ok(result.errors.includes('coverage-frontend: expected success, got skipped'));
  });

  it('accepts coverage jobs as skipped for documentation-only changes', () => {
    const result = evaluateRequiredChecks(input({ docs: 'true' }, { ...successResults, coverage: 'skipped', 'coverage-frontend': 'skipped' }));
    assert.equal(result.ok, true);
  });

  it('requires both reports for workflow changes that can affect either coverage job', () => {
    const result = evaluateRequiredChecks(input({ github: 'true' }));
    assert.equal(result.ok, true);
  });
});
