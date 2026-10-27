'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const { buildPayload, run, validateWorkflow, ghRequest, saveBackup } = require('./ci-quality-ruleset.cjs');
const { readFileSync, statSync, rmSync } = require('node:fs');
const { dirname } = require('node:path');
const approvedPolicy = require('../ci-quality-policy.json');

function snapshot() {
  return {
    id: 17001295, source_type: 'Repository', source: 'dallay/cortex',
    ...structuredClone(approvedPolicy),
    _links: { self: { href: 'ignored response metadata' } },
  };
}

test('adds exact App identities without mutating any existing policy or input', () => {
  const original = snapshot();
  const before = structuredClone(original);
  const payload = buildPayload(original);
  assert.deepEqual(Object.keys(payload).sort(), ['name', 'target', 'enforcement', 'conditions', 'bypass_actors', 'rules'].sort());
  assert.deepEqual(original, before);
  assert.equal(original.rules.length, 5);
  assert.equal(original.rules[4].parameters.required_status_checks.length, 12);
  const checks = payload.rules[4].parameters.required_status_checks;
  assert.deepEqual(checks.slice(-2), [
    { context: 'CodeQL', integration_id: 57789 },
    { context: 'SonarCloud Code Analysis', integration_id: 12526 },
  ]);
  checks.splice(-2);
  for (const field of Object.keys(payload)) assert.deepEqual(payload[field], original[field]);
});

test('is idempotent when both exact identities already exist', () => {
  const original = snapshot();
  const first = buildPayload(original);
  assert.deepEqual(buildPayload({ ...original, ...first }), first);
});

test('rejects missing, wrong, and ambiguous App identities for either external check', () => {
  for (const context of ['CodeQL', 'SonarCloud Code Analysis']) {
    for (const integration_id of [undefined, 1, null]) {
      const original = snapshot();
      original.rules[4].parameters.required_status_checks.push({ context, integration_id });
      assert.throws(() => buildPayload(original), /identity/);
    }
  }
  const original = snapshot();
  original.rules[4].parameters.required_status_checks.push(
    { context: 'CodeQL', integration_id: 57789 },
    { context: 'CodeQL', integration_id: 57789 },
  );
  assert.throws(() => buildPayload(original), /identity/);
});

test('fails closed on a different repository, target, branch, enforcement, or strict policy', () => {
  for (const change of [
    { id: 1 }, { source: 'someone/cortex' }, { source_type: 'Organization' },
    { target: 'tag' }, { enforcement: 'evaluate' },
    { conditions: { ref_name: { include: ['refs/heads/develop'], exclude: [] } } },
    { conditions: { ref_name: { include: ['~DEFAULT_BRANCH'], exclude: ['refs/heads/main'] } } },
  ]) assert.throws(() => buildPayload({ ...snapshot(), ...change }));
  for (const value of [false, undefined]) {
    const original = snapshot();
    original.rules[4].parameters.strict_required_status_checks_policy = value;
    assert.throws(() => buildPayload(original), /strict/);
  }
  for (const rules of [[], [snapshot().rules[4], snapshot().rules[4]]]) {
    assert.throws(() => buildPayload({ ...snapshot(), rules }), /one required_status_checks/);
  }
});

for (const [name, mutate] of Object.entries({
  'only a status rule with zero checks': (s) => {
    s.rules = [s.rules[4]];
    s.rules[0].parameters.required_status_checks = [];
  },
  'missing existing check': (s) => s.rules[4].parameters.required_status_checks.pop(),
  'all checks absent': (s) => { s.rules[4].parameters.required_status_checks = []; },
  'wrong existing App': (s) => { s.rules[4].parameters.required_status_checks[0].integration_id = 1; },
  'extra check': (s) => s.rules[4].parameters.required_status_checks.push({ context: 'Unexpected', integration_id: 15368 }),
  'duplicate existing check': (s) => s.rules[4].parameters.required_status_checks.push({ ...s.rules[4].parameters.required_status_checks[0] }),
  'missing protection rule': (s) => s.rules.splice(0, 1),
  'extra rule': (s) => s.rules.push({ type: 'creation' }),
  'duplicate rule': (s) => s.rules.push({ ...s.rules[0] }),
  'missing PR parameter': (s) => { delete s.rules[3].parameters.require_last_push_approval; },
  'changed PR protection': (s) => { s.rules[3].parameters.required_approving_review_count = 0; },
  'extra PR parameter': (s) => { s.rules[3].parameters.unapproved = true; },
  'missing status parameter': (s) => { delete s.rules[4].parameters.do_not_enforce_on_create; },
  'changed status parameter': (s) => { s.rules[4].parameters.do_not_enforce_on_create = true; },
  'changed bypass actor': (s) => { s.bypass_actors[1].actor_id = 1; },
  'missing bypass actor': (s) => s.bypass_actors.pop(),
  'extra bypass actor': (s) => s.bypass_actors.push({ ...s.bypass_actors[0] }),
  'extra condition': (s) => { s.conditions.repository_name = { include: ['other'] }; },
  'changed name': (s) => { s.name = 'unexpected'; },
})) {
  test(`rejects unapproved complete policy: ${name}`, () => {
    const original = snapshot();
    mutate(original);
    assert.throws(() => buildPayload(original));
  });
}

test('ignores response metadata but rejects extra parameters on an external identity', () => {
  const original = snapshot();
  assert.deepEqual(buildPayload({ ...original, updated_at: 'unfixed', node_id: 'unfixed' }), buildPayload(original));
  original.rules[4].parameters.required_status_checks.push({ context: 'CodeQL', integration_id: 57789, unapproved: true });
  assert.throws(() => buildPayload(original));
});

const workflow = `name: CI
on:
  pull_request:
  push:
    branches: [main]
jobs:
  sonar:
    runs-on: ubuntu-latest
    steps:
      - uses: SonarSource/sonarqube-scan-action@pinned
        with:
          args: >
            -Dsonar.qualitygate.wait=true
            -Dsonar.qualitygate.timeout=300
`;

const reviewedCondition = `    if: |
      always() &&
      needs.changes.result == 'success' &&
      needs.build-dashboard.result == 'success' &&
      (needs.test.result == 'success' || needs.test.result == 'skipped') &&
      (needs.test-frontend.result == 'success' || needs.test-frontend.result == 'skipped') && (
        github.event_name == 'pull_request' ||
        needs.changes.outputs.backend == 'true' ||
        needs.changes.outputs.frontend == 'true' ||
        needs.changes.outputs.github == 'true'
      )
`;

const publishedFiles = [
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
const mainSha = 'a'.repeat(40);

test('accepts the reviewed PR-first Sonar guard and main/develop branch scope only', () => {
  const reviewed = workflow.replace('  pull_request:', '  pull_request:\n    branches: [main, develop]')
    .replace('    runs-on:', reviewedCondition + '    runs-on:');
  assert.doesNotThrow(() => validateWorkflow(reviewed));
  assert.throws(() => validateWorkflow(reviewed.replace("github.event_name == 'pull_request' ||", "github.event_name == 'push' ||")));
});

function harness(options = {}) {
  let current = options.initialSnapshot || snapshot();
  const calls = [];
  const logs = [];
  const deps = {
    request(method, endpoint, payload) {
      calls.push({ method, endpoint, payload });
      if (options.httpError) throw new Error('gh API failed (HTTP or authentication error)');
      if (endpoint === 'repos/dallay/cortex') {
        return { full_name: 'dallay/cortex', default_branch: options.branch || 'main' };
      }
      if (endpoint === 'repos/dallay/cortex/git/ref/heads/main') {
        const changed = options.advanceMainAfterContentReads && calls.filter((call) => call.endpoint?.includes('/contents/')).length >= publishedFiles.length;
        return { object: { type: 'commit', sha: changed ? 'b'.repeat(40) : options.sha ?? mainSha } };
      }
      if (endpoint.includes('/contents/')) {
        const file = endpoint.match(/\/contents\/([^?]+)/)[1];
        if (file === options.missingFile) throw new Error('gh API failed: missing file');
        const content = file === options.staleFile ? 'old implementation'
          : file === '.github/workflows/ci.yml' ? options.remoteWorkflow ?? workflow : `reviewed ${file}`;
        return { encoding: 'base64', content: Buffer.from(content).toString('base64') };
      }
      if (method === 'PUT') {
        assert.ok(calls.some((call) => call.method === 'BACKUP'), 'backup must precede write');
        current = { ...current, ...structuredClone(payload) };
        if (options.mismatch) current.rules[0] = { type: 'non_fast_forward' };
        return current;
      }
      if (options.concurrent && calls.filter((call) => call.endpoint === endpoint).length > 1) {
        current.name = 'concurrent edit';
      }
      return structuredClone(current);
    },
    readLocalFile: (file) => Buffer.from(file === '.github/workflows/ci.yml' ? workflow : `reviewed ${file}`),
    saveBackup(value) { calls.push({ method: 'BACKUP', value }); return '/approved/snapshot.json'; },
    log: (value) => logs.push(value),
  };
  return { deps, calls, logs };
}

test('CLI defaults to GET-only dry-run with payload and additions', () => {
  const h = harness();
  const result = run([], h.deps);
  assert.equal(result.mode, 'dry-run');
  assert.deepEqual(result.added, ['CodeQL', 'SonarCloud Code Analysis']);
  assert.equal(result.payload.rules[4].parameters.required_status_checks.length, 14);
  assert.ok(h.calls.every((call) => call.method === 'GET'));
  assert.equal(run(['--dry-run'], harness().deps).mode, 'dry-run');
});

test('old main does not block dry-run, which never reads publication files', () => {
  const h = harness({ remoteWorkflow: 'old CI', missingFile: '.github/ci-path-filters.json' });
  assert.equal(run([], h.deps).mode, 'dry-run');
  assert.deepEqual(h.calls.map((call) => call.endpoint), [
    'repos/dallay/cortex', 'repos/dallay/cortex/rulesets/17001295',
  ]);
});

test('incomplete approved policy blocks apply before publication reads, backup or PUT', () => {
  const initialSnapshot = snapshot();
  initialSnapshot.rules = [initialSnapshot.rules[4]];
  initialSnapshot.rules[0].parameters.required_status_checks = [];
  const h = harness({ initialSnapshot });
  assert.throws(() => run(['--apply'], h.deps), /complete approved quality policy/);
  assert.equal(h.calls.length, 2);
  assert.ok(h.calls.every((call) => call.method === 'GET'));
});

test('invalid immutable main SHA blocks apply before contents reads or PUT', () => {
  for (const sha of ['', 'main', 'a'.repeat(39), 'z'.repeat(40)]) {
    const h = harness({ sha });
    assert.throws(() => run(['--apply'], h.deps), /immutable commit SHA/);
    assert.ok(!h.calls.some((call) => call.method === 'PUT' || call.endpoint?.includes('/contents/')));
  }
});

test('apply aborts before PUT if main advances after publication verification', () => {
  const h = harness({ advanceMainAfterContentReads: true });
  assert.throws(() => run(['--apply'], h.deps), /main advanced during activation/);
  assert.equal(h.calls.filter((call) => call.endpoint === 'repos/dallay/cortex/git/ref/heads/main').length, 2);
  assert.ok(!h.calls.some((call) => call.method === 'PUT'));
});

for (const [name, options] of Object.entries({
  'missing path filters': { missingFile: '.github/ci-path-filters.json' },
  'old required evaluator': { staleFile: '.github/scripts/ci-required.cjs' },
  'missing workflow test': { missingFile: '.github/scripts/ci-workflow.test.cjs' },
  'old ruleset helper': { staleFile: '.github/scripts/ci-quality-ruleset.cjs' },
  'old approved baseline': { staleFile: '.github/ci-quality-policy.json' },
})) {
  test(`blocks partial publication before PUT: ${name}`, () => {
    const h = harness(options);
    assert.throws(() => run(['--apply'], h.deps));
    assert.ok(!h.calls.some((call) => call.method === 'PUT'));
  });
}

test('resolves main twice and compares every executable CI artifact at the initial immutable SHA', () => {
  const h = harness();
  run(['--apply'], h.deps);
  const refReads = h.calls.filter((call) => call.endpoint === 'repos/dallay/cortex/git/ref/heads/main');
  assert.equal(refReads.length, 2);
  const firstRefRead = h.calls.indexOf(refReads[0]);
  const secondRefRead = h.calls.indexOf(refReads[1]);
  const contentReads = h.calls.filter((call) => call.endpoint?.includes('/contents/'));
  assert.ok(contentReads.every((call) => call.endpoint.endsWith(`?ref=${mainSha}`)));
  assert.ok(firstRefRead < h.calls.indexOf(contentReads[0]));
  assert.ok(contentReads.every((call) => h.calls.indexOf(call) < secondRefRead));
  const rulesetRead = h.calls.findIndex((call, index) => index > secondRefRead && call.method === 'GET' && call.endpoint === 'repos/dallay/cortex/rulesets/17001295');
  assert.ok(secondRefRead < rulesetRead);
  for (const file of publishedFiles) {
    const firstRead = h.calls.findIndex((call) => call.endpoint === `repos/dallay/cortex/contents/${file}?ref=${mainSha}`);
    assert.ok(firstRead >= 0 && firstRead < h.calls.findIndex((call) => call.method === 'PUT'), file);
  }
});

test('unknown, conflicting, and duplicate CLI arguments do not even read GitHub', () => {
  for (const args of [['--force'], ['--apply', '--dry-run'], ['--apply', '--apply'], ['--repo', 'someone/repo']]) {
    const h = harness();
    assert.throws(() => run(args, h.deps), /argument/);
    assert.deepEqual(h.calls, []);
  }
});

test('apply blocks wrong default branch, unpublished workflow, stale policy and HTTP errors', () => {
  for (const options of [
    { branch: 'develop' }, { remoteWorkflow: workflow + '# unpublished difference' },
    { concurrent: true }, { httpError: true },
  ]) {
    const h = harness(options);
    assert.throws(() => run(['--apply'], h.deps));
    assert.ok(h.calls.every((call) => call.method !== 'PUT'));
  }
});

test('workflow preflight rejects exemption, missing wait/timeout, PR filters and conditional Sonar', () => {
  assert.doesNotThrow(() => validateWorkflow(workflow));
  for (const invalid of [
    workflow + '# sonar_exempt',
    workflow.replace('wait=true', 'wait=false'),
    workflow.replace('timeout=300', 'timeout=600'),
    workflow.replace('  pull_request:', '  pull_request_target:'),
    workflow.replace('  pull_request:', '  pull_request:\n    paths: ["src/**"]'),
    workflow.replace('  pull_request:', '  pull_request:\n    branches: [release]'),
    workflow.replace('-Dsonar.qualitygate.wait=true', '# -Dsonar.qualitygate.wait=true'),
    workflow.replace('    runs-on:', '    if: needs.changes.outputs.backend == \'true\'\n    runs-on:'),
    workflow.replace('      - uses:', '      - if: github.actor != \'dependabot[bot]\'\n        uses:'),
  ]) assert.throws(() => validateWorkflow(invalid), /workflow/);
});

test('backup failure or invalid local policy stops apply before any PUT', () => {
  for (const failBackup of [true, false]) {
    const h = harness();
    if (failBackup) h.deps.saveBackup = () => { throw new Error('disk full'); };
    else {
      const readLocalFile = h.deps.readLocalFile;
      h.deps.readLocalFile = (file) => file === '.github/workflows/ci.yml'
        ? Buffer.from(workflow.replace('wait=true', 'wait=false')) : readLocalFile(file);
    }
    assert.throws(() => run(['--apply'], h.deps));
    assert.ok(!h.calls.some((call) => call.method === 'PUT'));
  }
});

test('already activated ruleset is a verified no-op, not another write', () => {
  const h = harness();
  const request = h.deps.request;
  h.deps.request = (method, endpoint, payload) => {
    const value = request(method, endpoint, payload);
    return endpoint.endsWith('/rulesets/17001295') ? { ...value, ...buildPayload(value) } : value;
  };
  assert.equal(run(['--apply'], h.deps).mode, 'unchanged');
  assert.ok(h.calls.every((call) => call.method === 'GET'));
});

test('explicit apply saves original snapshot, uses PUT and verifies all prior fields', () => {
  const h = harness();
  const result = run(['--apply'], h.deps);
  assert.equal(result.mode, 'applied');
  assert.deepEqual(h.calls.find((call) => call.method === 'BACKUP').value, snapshot());
  const put = h.calls.find((call) => call.method === 'PUT');
  assert.equal(put.endpoint, 'repos/dallay/cortex/rulesets/17001295');
  assert.deepEqual(put.payload, buildPayload(snapshot()));
  assert.equal(h.calls.at(-1).method, 'GET');
});

test('post-write mismatch fails without blind rollback', () => {
  const h = harness({ mismatch: true });
  assert.throws(() => run(['--apply'], h.deps), /verification.*manual/i);
  assert.equal(h.calls.filter((call) => call.method === 'PUT').length, 1);
  assert.ok(h.logs.some((line) => line.includes('/approved/snapshot.json')));
});

test('gh adapter uses explicit hostname, argv and stdin, never a shell or raw error output', () => {
  let invocation;
  const execute = (...args) => { invocation = args; return { status: 0, stdout: '{}' }; };
  ghRequest('PUT', 'repos/dallay/cortex/rulesets/17001295', { name: '$(not a command)' }, execute);
  assert.equal(invocation[0], 'gh');
  assert.deepEqual(invocation[1], [
    'api', '--hostname', 'github.com', '--method', 'PUT',
    'repos/dallay/cortex/rulesets/17001295', '--input', '-',
  ]);
  assert.equal(invocation[2].shell, false);
  assert.deepEqual(JSON.parse(invocation[2].input), { name: '$(not a command)' });
  assert.throws(() => ghRequest('GET', 'x', undefined, () => ({ status: 1, stderr: 'SECRET' })),
    (error) => !error.message.includes('SECRET') && /gh API/.test(error.message));
});

test('backup persists the complete snapshot in a private temporary directory', () => {
  const path = saveBackup(snapshot());
  try {
    assert.deepEqual(JSON.parse(readFileSync(path, 'utf8')), snapshot());
    assert.equal(statSync(path).mode & 0o777, 0o600);
    assert.equal(statSync(dirname(path)).mode & 0o777, 0o700);
    assert.match(path, /opencode\/ci-quality-ruleset-[^/]+\/ruleset-before\.json$/);
  } finally {
    rmSync(dirname(path), { recursive: true });
  }
});
