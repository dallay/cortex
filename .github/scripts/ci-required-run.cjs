const { evaluateRequiredChecks } = require('./ci-required.cjs');

const resultKeys = {
  changes: 'CHANGES',
  'build-dashboard': 'BUILD_DASHBOARD',
  fmt: 'FMT',
  markdown: 'MARKDOWN',
  'test-labeler': 'TEST_LABELER',
  clippy: 'CLIPPY',
  check: 'CHECK',
  test: 'TEST',
  'test-frontend': 'TEST_FRONTEND',
  'test-e2e': 'TEST_E2E',
  doc: 'DOC',
  audit: 'AUDIT',
  'trivy-fs': 'TRIVY_FS',
  'gitleaks-pr': 'GITLEAKS_PR',
  'semgrep-pr': 'SEMGREP_PR',
  sonar: 'SONAR',
};
const outputKeys = {
  backend: 'CHANGES_BACKEND',
  frontend: 'CHANGES_FRONTEND',
  docker: 'CHANGES_DOCKER',
  github: 'CHANGES_GITHUB',
  docs: 'CHANGES_DOCS',
  labeler: 'CHANGES_LABELER',
};

const results = Object.fromEntries(Object.entries(resultKeys).map(([job, envKey]) => [job, process.env[`RESULT_${envKey}`] || 'missing']));
const changes = Object.fromEntries(Object.entries(outputKeys).map(([key, envKey]) => [key, process.env[envKey] || 'false']));
const outcome = evaluateRequiredChecks({
  eventName: process.env.EVENT_NAME,
  actor: process.env.ACTOR,
  fork: process.env.IS_FORK === 'true',
  sonarTokenPresent: process.env.SONAR_TOKEN_PRESENT === 'true',
  changes,
  results,
});

for (const [job, result] of Object.entries(results)) console.log(`${job}: ${result}`);
if (!outcome.ok) {
  for (const error of outcome.errors) console.error(`::error::${error}`);
  process.exitCode = 1;
} else {
  console.log('CI / Required passed: every applicable check succeeded; all skipped jobs are justified.');
}
