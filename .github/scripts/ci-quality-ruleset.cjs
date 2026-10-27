'use strict';

const { isDeepStrictEqual } = require('node:util');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { readFileSync, mkdirSync, mkdtempSync, writeFileSync } = require('node:fs');
const { join, resolve } = require('node:path');
const { tmpdir } = require('node:os');
const APPROVED_POLICY = require('../ci-quality-policy.json');

const REPO = 'dallay/cortex';
const RULESET_ID = 17001295;
const FIELDS = ['name', 'target', 'enforcement', 'conditions', 'bypass_actors', 'rules'];
const EXTERNAL_CHECKS = [
  { context: 'CodeQL', integration_id: 57789 },
  { context: 'SonarCloud Code Analysis', integration_id: 12526 },
];
const PUBLICATION_FILES = [
  '.github/workflows/ci.yml',
  '.github/ci-path-filters.json',
  '.github/scripts/ci-required.cjs',
  '.github/scripts/ci-required-run.cjs',
  '.github/scripts/ci-required.test.cjs',
  '.github/scripts/ci-workflow.test.cjs',
  '.github/scripts/ci-quality-ruleset.cjs',
  '.github/scripts/ci-quality-ruleset.test.cjs',
  '.github/ci-quality-policy.json',
];

function updateFields(snapshot) {
  return Object.fromEntries(FIELDS.map((field) => [field, structuredClone(snapshot[field])]));
}

function buildPayload(snapshot) {
  if (snapshot.id !== RULESET_ID || snapshot.source_type !== 'Repository' || snapshot.source !== REPO
    || snapshot.target !== 'branch' || snapshot.enforcement !== 'active') {
    throw new Error('Expected active branch ruleset 17001295 from Repository dallay/cortex');
  }
  const refs = snapshot.conditions?.ref_name;
  if (!refs || !isDeepStrictEqual(refs.exclude, []) || refs.include?.length !== 1
    || !['~DEFAULT_BRANCH', 'refs/heads/main'].includes(refs.include[0])) {
    throw new Error('Expected ruleset scoped only to the default branch main, without exclusions');
  }
  const required = snapshot.rules?.filter((rule) => rule.type === 'required_status_checks');
  if (required?.length !== 1) throw new Error('Expected exactly one required_status_checks rule');
  if (required[0].parameters?.strict_required_status_checks_policy !== true) {
    throw new Error('Expected strict required status checks');
  }
  if (!Array.isArray(required[0].parameters.required_status_checks)) throw new Error('Missing checks');
  const payload = updateFields(snapshot);
  const checks = payload.rules.find((rule) => rule.type === 'required_status_checks').parameters.required_status_checks;
  for (const expected of EXTERNAL_CHECKS) {
    const matches = checks.filter((check) => check.context === expected.context);
    if (matches.length > 1 || (matches.length === 1 && !isDeepStrictEqual(matches[0], expected))) {
      throw new Error(`Conflicting or ambiguous App identity: ${expected.context}`);
    }
    if (matches.length === 0) checks.push({ ...expected });
  }
  const baseline = updateFields(snapshot);
  const parameters = baseline.rules.find((rule) => rule.type === 'required_status_checks').parameters;
  parameters.required_status_checks = parameters.required_status_checks
    .filter((check) => !EXTERNAL_CHECKS.some((external) => external.context === check.context));
  // Exact array comparison deliberately preserves order and multiplicity: no deduplication
  // or sorting may hide a missing, duplicated, altered, or unexpected protection.
  if (!isDeepStrictEqual(baseline, APPROVED_POLICY)) {
    throw new Error('Ruleset differs from the complete approved quality policy; review required');
  }
  return payload;
}

// Deliberately conservative preflight for the reviewed, block-style CI workflow.
// This is not a YAML parser. Unsupported formatting must fail closed, not bypass checks.
function validateWorkflow(text) {
  if (/sonar_exempt/.test(text)) throw new Error('CI workflow still contains sonar_exempt');
  // Comments cannot serve as evidence that an executable setting is present.
  text = text.split('\n').map((line) => line.replace(/\s*#.*$/, '')).join('\n');
  const prOptions = text.match(/^  pull_request:[ \t]*\r?\n([\s\S]*?)(?=^\S|^  \S|$(?![\s\S]))/m)?.[1];
  const sonar = text.match(/^  sonar:\s*\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/m)?.[1];
  const guard = sonar?.match(/^    if: \|\n((?:^      .*\n)+)/m);
  // Permit only the exact reviewed PR-first guard, not arbitrary expressions containing a PR marker.
  const reviewedGuard = "always() && needs.changes.result == 'success' && needs.build-dashboard.result == 'success' && "
    + "(needs.test.result == 'success' || needs.test.result == 'skipped') && "
    + "(needs.test-frontend.result == 'success' || needs.test-frontend.result == 'skipped') && ( "
    + "github.event_name == 'pull_request' || needs.changes.outputs.backend == 'true' || "
    + "needs.changes.outputs.frontend == 'true' || needs.changes.outputs.github == 'true' )";
  const reviewed = guard && guard[1].trim().replace(/\s+/g, ' ') === reviewedGuard;
  const remaining = reviewed ? sonar.replace(guard[0], '') : sonar;
  const conditionals = remaining?.match(/^\s+(?:- )?if:.*$/gm) || [];
  const branches = prOptions?.trim();
  if (!/^  pull_request:\s*$/m.test(text)
    || (branches && !/^branches: \[main(?:, develop)?\]$/.test(branches)) || /pull_request_target|sonar_exempt/.test(text)
    || /^\s+(?:paths|paths-ignore|branches-ignore|types):/m.test(text)
    || !sonar || !/uses: SonarSource\/sonarqube-scan-action@/.test(sonar)
    || !/-Dsonar\.qualitygate\.wait=true(?:\s|$)/.test(sonar)
    || !/-Dsonar\.qualitygate\.timeout=300(?:\s|$)/.test(sonar)
    || /needs\.[\w-]+\.outputs/.test(remaining)
    || conditionals.some((line) => !/^\s+if:\s*\$\{\{\s*(?:always\(\) && )?!cancelled\(\)\s*\}\}\s*$/.test(line))) {
    throw new Error('CI workflow preflight failed: require every PR, unconditional Sonar scan and quality gate wait=true timeout=300');
  }
}

function ghRequest(method, endpoint, payload, execute = spawnSync) {
  const args = ['api', '--hostname', 'github.com', '--method', method, endpoint];
  if (payload !== undefined) args.push('--input', '-');
  const result = execute('gh', args, {
    shell: false, encoding: 'utf8', input: payload === undefined ? undefined : JSON.stringify(payload),
    timeout: 30000, maxBuffer: 4 * 1024 * 1024,
  });
  // Do not print gh stderr, which can contain environment-specific sensitive details.
  if (result.error || result.status !== 0) throw new Error('gh API failed (HTTP, authentication or process error)');
  try { return JSON.parse(result.stdout); } catch { throw new Error('gh API returned invalid JSON'); }
}

function saveBackup(snapshot) {
  const root = join(tmpdir(), 'opencode');
  mkdirSync(root, { recursive: true, mode: 0o700 });
  const directory = mkdtempSync(join(root, 'ci-quality-ruleset-'));
  const path = join(directory, 'ruleset-before.json');
  writeFileSync(path, JSON.stringify(snapshot, null, 2) + '\n', { mode: 0o600, flag: 'wx' });
  return path;
}

function run(argv, dependencies = {}) {
  if (argv.length > 1 || (argv.length === 1 && !['--apply', '--dry-run'].includes(argv[0]))) {
    throw new Error('Unknown or conflicting argument; use no arguments, --dry-run, or --apply');
  }
  const request = dependencies.request || ghRequest;
  const log = dependencies.log || console.log;
  const repository = request('GET', `repos/${REPO}`);
  if (repository.full_name !== REPO || repository.default_branch !== 'main') {
    throw new Error('Expected repository dallay/cortex with default branch main');
  }
  const endpoint = `repos/${REPO}/rulesets/${RULESET_ID}`;
  const snapshot = request('GET', endpoint);
  const payload = buildPayload(snapshot);
  const priorChecks = snapshot.rules.find((rule) => rule.type === 'required_status_checks').parameters.required_status_checks;
  const added = EXTERNAL_CHECKS.filter((check) => !priorChecks.some((prior) => prior.context === check.context))
    .map((check) => check.context);
  if (argv[0] !== '--apply') return { mode: 'dry-run', repository: REPO, ruleset: RULESET_ID, added, payload };

  const readLocalFile = dependencies.readLocalFile || ((file) => readFileSync(resolve(__dirname, '../..', file)));
  const localFiles = new Map(PUBLICATION_FILES.map((file) => [file, readLocalFile(file)]));
  validateWorkflow(localFiles.get('.github/workflows/ci.yml').toString('utf8'));
  // Resolve the moving branch exactly once. Every remote file read uses this commit.
  const ref = request('GET', `repos/${REPO}/git/ref/heads/main`);
  const publishedSha = ref.object?.sha;
  if (ref.object?.type !== 'commit' || !/^[a-f0-9]{40}$/.test(publishedSha || '')) {
    throw new Error('Cannot resolve remote main to an immutable commit SHA');
  }
  const hash = (content) => createHash('sha256').update(content).digest('hex');
  const verifyPublished = () => {
    for (const [path, local] of localFiles) {
      const file = request('GET', `repos/${REPO}/contents/${path}?ref=${publishedSha}`);
      if (file.encoding !== 'base64' || typeof file.content !== 'string'
        || hash(Buffer.from(file.content, 'base64')) !== hash(local)) {
        throw new Error(`Incomplete CI publication at ${publishedSha}: ${path}; publish before activation`);
      }
    }
  };
  verifyPublished();
  if (added.length === 0) return { mode: 'unchanged', publishedSha, added, payload };
  const backup = (dependencies.saveBackup || saveBackup)(snapshot);
  log(`Ruleset snapshot saved: ${backup}`);
  // Re-resolve main to reduce the stale-write window; the API has no atomic compare-and-swap.
  const currentRef = request('GET', `repos/${REPO}/git/ref/heads/main`);
  if (currentRef.object?.type !== 'commit' || currentRef.object.sha !== publishedSha) {
    throw new Error('Remote main advanced during activation; no update sent. Review and retry');
  }
  const latest = request('GET', endpoint);
  buildPayload(latest);
  if (!isDeepStrictEqual(updateFields(latest), updateFields(snapshot))) {
    throw new Error('Ruleset changed concurrently; no update sent. Review and retry');
  }
  try {
    request('PUT', endpoint, payload);
    const actual = request('GET', endpoint);
    buildPayload(actual);
    if (!isDeepStrictEqual(updateFields(actual), payload)) throw new Error('Ruleset mismatch');
  } catch {
    throw new Error(`Apply or verification failed; manual inspection required. No automatic rollback. Snapshot: ${backup}`);
  }
  return { mode: 'applied', publishedSha, backup, added, payload };
}

if (require.main === module) {
  try { console.log(JSON.stringify(run(process.argv.slice(2)), null, 2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}

module.exports = { buildPayload, run, validateWorkflow, ghRequest, saveBackup };
