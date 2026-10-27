const EXPECTED = [
  { job: 'changes', when: () => true },
  { job: 'ci-contract', when: () => true },
  { job: 'doctor', when: (c) => c.doctor || c.github },
  { job: 'build-dashboard', when: (c) => c.pullRequest || c.backend || c.frontend || c.github },
  { job: 'fmt', when: (c) => c.backend || c.github },
  { job: 'markdown', when: (c) => c.docs || c.github },
  { job: 'test-labeler', when: (c) => c.labeler || c.github },
  { job: 'clippy', when: (c) => c.backend || c.github },
  { job: 'check', when: (c) => c.backend || c.github },
  { job: 'test', when: (c) => c.backend || c.github },
  { job: 'test-frontend', when: (c) => c.frontendTests },
  { job: 'coverage', when: (c) => c.backend || c.github },
  { job: 'coverage-frontend', when: (c) => c.frontend || c.github },
  { job: 'test-e2e', when: (c) => c.backend || c.frontend || c.docker || c.github },
  { job: 'doc', when: (c) => c.backend || c.github },
  { job: 'audit', when: (c) => c.backend || c.github },
  { job: 'trivy-fs', when: (c) => c.backend || c.frontend || c.docker || c.github },
  { job: 'gitleaks-pr', when: () => true },
  { job: 'semgrep-pr', when: (c) => c.backend || c.docker || c.github },
  { job: 'sonar', when: (c) => c.pullRequest || c.backend || c.frontend || c.github },
];

function evaluateRequiredChecks(input) {
  const changes = input.changes || {};
  const errors = [];
  const selected = {};
  for (const key of ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs', 'labeler']) {
    if (!Object.hasOwn(changes, key) || (changes[key] !== 'true' && changes[key] !== 'false')) {
      errors.push(`changes.${key}: expected true or false, got ${String(changes[key])}`);
    }
    selected[key] = changes[key] === 'true';
  }
  selected.frontendTests = selected.frontend || selected.github;
  selected.pullRequest = input.eventName === 'pull_request';

  for (const { job, when } of EXPECTED) {
    const result = input.results?.[job] || 'missing';
    const expected = when(selected);
    if (expected && result !== 'success') {
      errors.push(`${job}: expected success, got ${result}`);
    } else if (!expected && result !== 'skipped' && result !== 'success') {
      errors.push(`${job}: expected success or legitimate skipped, got ${result}`);
    }
  }

  const sonarRequired = selected.pullRequest || selected.backend || selected.frontend || selected.github;
  if (sonarRequired && !input.sonarTokenPresent) {
    errors.push('sonar: SONAR_TOKEN is required for this analysis');
  }

  return { ok: errors.length === 0, errors };
}

module.exports = { evaluateRequiredChecks };
