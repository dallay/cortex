const EXPECTED = [
  { job: 'changes', when: () => true },
  { job: 'test-ci-policy', when: (c) => c.github },
  { job: 'build-dashboard', when: (c) => c.backend || c.frontend || c.github },
  { job: 'fmt', when: (c) => c.backend || c.github },
  { job: 'markdown', when: (c) => c.docs || c.github },
  { job: 'test-labeler', when: (c) => c.labeler || c.github },
  { job: 'clippy', when: (c) => c.backend || c.github },
  { job: 'check', when: (c) => c.backend || c.github },
  { job: 'test', when: (c) => c.backend || c.github },
  { job: 'test-frontend', when: (c) => c.frontendTests },
  { job: 'test-e2e', when: (c) => c.backend || c.frontend || c.docker || c.github },
  { job: 'doc', when: (c) => c.backend || c.github },
  { job: 'audit', when: (c) => c.backend || c.github },
  { job: 'trivy-fs', when: (c) => c.backend || c.docker || c.github },
  { job: 'gitleaks-pr', when: () => true },
  { job: 'semgrep-pr', when: (c) => c.backend || c.docker || c.github },
  { job: 'sonar', when: (c) => c.backend || c.frontend || c.github },
];

function evaluateRequiredChecks(input) {
  const changes = input.changes || {};
  const selected = Object.fromEntries(Object.entries(changes).map(([key, value]) => [key, value === 'true']));
  selected.frontendTests = selected.frontend || selected.github;
  const errors = [];

  for (const { job, when } of EXPECTED) {
    const result = input.results?.[job] || 'missing';
    const expected = when(selected);
    if (expected && result !== 'success') {
      errors.push(`${job}: expected success, got ${result}`);
    } else if (!expected && result !== 'skipped' && result !== 'success') {
      errors.push(`${job}: expected success or legitimate skipped, got ${result}`);
    }
  }

  const sonarRequired = selected.backend || selected.frontend || selected.github;
  const sonarExempt = input.eventName === 'pull_request' && (input.fork === true || input.actor === 'dependabot[bot]');
  if (sonarRequired && !input.sonarTokenPresent && !sonarExempt) {
    errors.push('sonar: SONAR_TOKEN is required for this analysis');
  }
  if (sonarRequired && sonarExempt && input.results?.sonar !== 'success') {
    errors.push(`sonar: fork/Dependabot exception requires SonarCloud job success, got ${input.results?.sonar || 'missing'}`);
  }

  return { ok: errors.length === 0, errors };
}

module.exports = { evaluateRequiredChecks };
