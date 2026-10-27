'use strict';

const { describe, it } = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const { evaluateRequiredChecks } = require('./ci-required.cjs');

const categories = ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs', 'labeler'];
const noChanges = Object.fromEntries(categories.map((key) => [key, 'false']));

const successResults = {
  changes: 'success',
  'ci-contract': 'success',
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
  doctor: 'success',
};

function input(changes, results = successResults) {
  return {
    eventName: 'pull_request',
    actor: 'developer',
    fork: false,
    sonarTokenPresent: true,
    changes: { ...noChanges, ...changes },
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

describe('resultados aplicables', () => {
  const allChanges = Object.fromEntries(categories.map((key) => [key, 'true']));
  for (const job of Object.keys(successResults)) {
    for (const status of ['failure', 'cancelled', 'missing', 'skipped', 'neutral', undefined]) {
      it(`bloquea ${job}: ${String(status)}`, () => {
        const results = { ...successResults, [job]: status };
        if (status === undefined) delete results[job];
        const outcome = evaluateRequiredChecks(input(allChanges, results));
        assert.equal(outcome.ok, false);
        assert.ok(outcome.errors.includes(`${job}: expected success, got ${status || 'missing'}`));
      });
    }
  }
});

describe('Sonar obligatorio en toda PR', () => {
  for (const changes of [{ docs: 'true' }, {}]) {
    for (const identity of [{ actor: 'developer', fork: false }, { actor: 'developer', fork: true }, { actor: 'dependabot[bot]', fork: false }]) {
      for (const job of ['sonar', 'build-dashboard']) {
        it(`bloquea ${job} omitido: ${JSON.stringify({ changes, ...identity })}`, () => {
          const candidate = { ...input(changes, { ...successResults, [job]: 'skipped' }), ...identity };
          const outcome = evaluateRequiredChecks(candidate);
          assert.equal(outcome.ok, false);
          assert.ok(outcome.errors.includes(`${job}: expected success, got skipped`));
        });
      }
      it(`bloquea token ausente: ${JSON.stringify({ changes, ...identity })}`, () => {
        const candidate = { ...input(changes), ...identity, sonarTokenPresent: false };
        const outcome = evaluateRequiredChecks(candidate);
        assert.equal(outcome.ok, false);
        assert.ok(outcome.errors.includes('sonar: SONAR_TOKEN is required for this analysis'));
      });
    }
  }

  it('permite skips de Sonar y dashboard en push de documentación', () => {
    const candidate = input({ docs: 'true' }, { ...successResults, sonar: 'skipped', 'build-dashboard': 'skipped' });
    candidate.eventName = 'push';
    candidate.sonarTokenPresent = false;
    assert.equal(evaluateRequiredChecks(candidate).ok, true);
  });
});

describe('contrato de categorías', () => {
  for (const category of categories) {
    for (const value of [undefined, '', true, false, 'TRUE', 'False', '0', null]) {
      it(`bloquea ${category} con valor ${String(value)} (${typeof value})`, () => {
        const candidate = input({ [category]: value });
        if (value === undefined) delete candidate.changes[category];
        const result = evaluateRequiredChecks(candidate);
        assert.equal(result.ok, false);
        assert.ok(result.errors.some((error) => error.startsWith(`changes.${category}:`)));
      });
    }
  }

  it('no convierte salidas ausentes del entorno a false', () => {
    const env = { ...process.env, EVENT_NAME: 'push', SONAR_TOKEN_PRESENT: 'true' };
    for (const key of Object.keys(env)) {
      if (key.startsWith('CHANGES_') || key.startsWith('RESULT_')) delete env[key];
    }
    for (const job of Object.keys(successResults)) {
      env[`RESULT_${job.replaceAll('-', '_').toUpperCase()}`] = 'success';
    }
    const child = spawnSync(process.execPath, [require.resolve('./ci-required-run.cjs')], { env, encoding: 'utf8' });
    assert.equal(child.status, 1);
    for (const category of categories) assert.match(child.stderr, new RegExp(`changes\\.${category}:`));
  });
});

describe('doctor test check', () => {
  it('requires the doctor test when doctor inputs change', () => {
    const result = evaluateRequiredChecks(input({ doctor: 'true' }, { ...successResults, doctor: 'success' }));
    assert.equal(result.ok, true);
  });

  it('fails the gate when the doctor test is skipped for doctor changes', () => {
    const result = evaluateRequiredChecks(input({ doctor: 'true' }, { ...successResults, doctor: 'skipped' }));
    assert.equal(result.ok, false);
    assert.ok(result.errors.includes('doctor: expected success, got skipped'));
  });

  it('allows the doctor test to skip for documentation-only changes', () => {
    const result = evaluateRequiredChecks(input({ docs: 'true' }, { ...successResults, doctor: 'skipped' }));
    assert.equal(result.ok, true);
  });
});
