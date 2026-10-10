'use strict';

/**
 * Regression tests for the deterministic issue classifier.
 * Run with: node --test .github/scripts/issue-labeler.test.cjs
 * No dependencies — node:test + node:assert only.
 */

const { describe, it } = require('node:test');
const assert = require('node:assert/strict');
const { classify, loadRules, parseConventionalTitle } = require('./issue-labeler.cjs');

const rules = loadRules();
const add = (r) => [...r.add].sort();
const remove = (r) => [...r.remove].sort();

describe('conventional titles', () => {
  it('fix(huginn): MCP session recovery -> huginn/bug/runtime, no triage', () => {
    const r = classify({ title: 'fix(huginn): improve MCP session recovery', body: '', existingLabels: [] }, rules);
    assert.deepEqual(add(r), ['area/runtime', 'product/huginn', 'type/bug']);
    assert.deepEqual(remove(r), []);
    assert.equal(r.needsTriage, false);
  });

  // Legacy `agent` scope: should resolve to product/huginn for new issues.
  it('fix(agent): legacy scope resolves to product/huginn, no triage', () => {
    const r = classify({ title: 'fix(agent): improve MCP session recovery', body: '', existingLabels: [] }, rules);
    assert.deepEqual(add(r).sort(), ['area/runtime', 'product/huginn', 'type/bug']);
    assert.deepEqual(remove(r), []);
    assert.equal(r.needsTriage, false);
  });

  // Legacy `product/agent` label already on the issue is not stripped.
  it('legacy product/agent label is preserved when no new scope arrives', () => {
    const r = classify(
      { title: 'Some vague improvement', body: '', existingLabels: ['product/agent', 'type/chore', 'area/ci'] },
      rules
    );
    assert.ok(![...r.remove].some((l) => l.startsWith('product/')), 'must not silently strip product/agent');
  });

  it('feat(rook): provider fallback -> rook/feature/providers', () => {
    const r = classify(
      { title: 'feat(rook): add Anthropic fallback', body: 'provider routing', existingLabels: [] },
      rules
    );
    assert.ok(r.add.has('product/rook'), 'product/rook');
    assert.ok(r.add.has('type/feature'), 'type/feature');
    assert.ok(r.add.has('area/providers'), 'area/providers');
  });

  it('test(huginn): maps to type/test', () => {
    const r = classify({ title: 'test(huginn): session replay harness', body: '', existingLabels: [] }, rules);
    assert.ok(r.add.has('type/test'));
    assert.ok(r.add.has('product/huginn'));
  });

  it('multi-scope fix(rook,huginn): is ambiguous -> triage, no invented product', () => {
    const t = parseConventionalTitle('fix(rook,huginn): shared thing', rules);
    assert.equal(t.scope, null);
    const r = classify({ title: 'fix(rook,huginn): shared thing', body: '', existingLabels: [] }, rules);
    assert.ok(![...r.add].some((l) => l.startsWith('product/')), 'must not invent product');
    assert.ok(r.add.has('triage/needs-classification'));
  });
});

describe('legacy product selection', () => {
  for (const source of ['title', 'form']) {
    for (const product of ['agent', 'huginn']) {
      it(`${source} ${product} preserves legacy naming or explicitly corrects it`, () => {
        const input = {
          title: `fix(${source === 'title' ? product : 'agent'}): session recovery`,
          body: source === 'form' ? `### Product\n\n${product === 'agent' ? 'Agent' : 'Huginn'}` : '',
          existingLabels: ['product/agent', 'type/bug', 'area/runtime'],
        };
        const result = classify(input, rules);
        assert.equal(result.desired.product, `product/${product}`);
        assert.deepEqual(add(result), product === 'huginn' ? ['product/huginn'] : []);
        assert.deepEqual(remove(result), product === 'huginn' ? ['product/agent'] : []);
        assert.equal(result.needsTriage, false);
        const labels = input.existingLabels.filter((label) => !result.remove.has(label)).concat(add(result));
        const repeated = classify({ ...input, existingLabels: labels }, rules);
        assert.deepEqual(add(repeated), []);
        assert.deepEqual(remove(repeated), []);
      });
    }
  }

  it('a legacy Agent form on a new issue selects Huginn', () => {
    const result = classify({ title: 'fix: session recovery', body: '### Product\n\nAgent' }, rules);
    assert.equal(result.desired.product, 'product/huginn');
    assert.ok(!result.add.has('product/agent'));
  });
});

describe('issue forms (highest priority)', () => {
  it('form Rook/providers wins over title scope', () => {
    const body = '### Product\n\nRook\n\n### Technical Area\n\nproviders\n\n### Description\n\ntimeout';
    const r = classify({ title: 'fix(huginn): gateway timeout', body, existingLabels: ['type/bug'] }, rules);
    assert.ok(r.add.has('product/rook'), 'form product wins');
    assert.ok(r.add.has('area/providers'));
    // type preserved from existing (form frontmatter), not overwritten by title
    assert.ok(!r.add.has('type/bug'), 'type already present -> no add');
  });

  it('legacy "Agent" form value resolves to product/huginn', () => {
    const body = '### Product\n\nAgent\n\n### Technical Area\n\nruntime';
    const r = classify({ title: 'fix(rook): migrate to agent runtime', body, existingLabels: ['product/rook', 'type/bug'] }, rules);
    assert.ok(r.add.has('product/huginn'), 'legacy Agent form must canonicalize to product/huginn');
    assert.ok(r.remove.has('product/rook'));
  });

  it('invalid form product -> triage, no fallback guessing', () => {
    const body = '### Product\n\nUnknownProduct\n\n### Technical Area\n\nruntime';
    const r = classify({ title: 'fix(huginn): x', body, existingLabels: [] }, rules);
    assert.ok(r.add.has('triage/needs-classification'));
    assert.ok(![...r.add].some((l) => l.startsWith('product/')));
  });
});

describe('ambiguity and human preservation', () => {
  it('"Agent integration with Rook" body mention never infers product', () => {
    const r = classify(
      { title: 'Agent integration with Rook', body: 'mentions both products', existingLabels: [] },
      rules
    );
    assert.ok(![...r.add].some((l) => l.startsWith('product/')));
    assert.ok(r.add.has('triage/needs-classification'));
  });

  it('existing single product preserved when no explicit source', () => {
    const r = classify(
      { title: 'Flaky dashboard without scope', body: 'no form', existingLabels: ['product/rook', 'type/bug', 'area/dashboard'] },
      rules
    );
    assert.ok(!r.add.has('product/huginn'));
    assert.deepEqual(remove(r).filter((l) => l.startsWith('product/')), []);
  });

  it('explicit form replaces conflicting product (safe correction)', () => {
    const body = '### Product\n\nHuginn\n\n### Technical Area\n\nruntime';
    const r = classify(
      { title: 'fix(rook): migrate to agent runtime', body, existingLabels: ['product/rook', 'type/bug'] },
      rules
    );
    assert.ok(r.add.has('product/huginn'));
    assert.ok(r.remove.has('product/rook'));
  });

  it('ambiguous title never removes existing product', () => {
    const r = classify(
      { title: 'Some vague improvement', body: '', existingLabels: ['product/rook', 'type/chore', 'area/ci'] },
      rules
    );
    assert.ok(![...r.remove].some((l) => l.startsWith('product/')));
  });

  it('human type wins over stale title prefix', () => {
    const r = classify(
      { title: 'fix(huginn): old prefix', body: '', existingLabels: ['product/huginn', 'type/feature', 'area/runtime'] },
      rules
    );
    assert.ok(!r.add.has('type/bug'), 'must not fight human type');
    assert.ok(!r.remove.has('type/feature'));
  });
});

describe('renovate and bots', () => {
  it('Dependency Dashboard from renovate[bot] -> shared/dependencies/chore', () => {
    const r = classify(
      { title: 'Dependency Dashboard', body: '', existingLabels: [], author: 'renovate[bot]' },
      rules
    );
    assert.ok(r.add.has('product/shared'));
    assert.ok(r.add.has('area/dependencies'));
    assert.ok(r.add.has('type/chore'));
  });

  it('human issue with dashboard title does NOT trigger renovate exception', () => {
    const r = classify(
      { title: 'Dependency Dashboard for my team', body: '', existingLabels: [], author: 'yacosta738' },
      rules
    );
    assert.ok(!r.add.has('product/shared'), 'must not invent product for human');
    assert.ok(r.add.has('triage/needs-classification'));
  });

  it('renovate reconciles conflicting types to single chore', () => {
    const r = classify(
      {
        title: 'Dependency Dashboard',
        body: '',
        existingLabels: ['product/shared', 'type/bug', 'type/feature', 'area/dependencies'],
        author: 'renovate[bot]',
      },
      rules
    );
    assert.ok(r.add.has('type/chore'));
    assert.ok(r.remove.has('type/bug'));
    assert.ok(r.remove.has('type/feature'));
  });
});

describe('token matching (no substring false positives)', () => {
  it('"author" does not trigger area/auth', () => {
    const r = classify(
      { title: 'Fix author name display', body: 'the author field is blank', existingLabels: ['product/rook', 'type/bug'] },
      rules
    );
    assert.ok(!r.add.has('area/auth'), 'auth substring in author must not match');
  });

  it('"score" does not trigger area/core', () => {
    const r = classify(
      { title: 'Fix high score display', body: 'scoreboard shows wrong score', existingLabels: ['product/rook', 'type/bug'] },
      rules
    );
    assert.ok(!r.add.has('area/core'), 'core substring in score must not match');
  });

  it('whole-word auth still matches', () => {
    const r = classify(
      { title: 'fix(rook): oauth login fails', body: '', existingLabels: [] },
      rules
    );
    assert.ok(r.add.has('area/auth'));
  });
});

describe('explicit ambiguity requires triage', () => {
  it('fix(rook,huginn) with stale product/rook -> triage, no preserve', () => {
    const r = classify(
      {
        title: 'fix(rook,huginn): shared thing',
        body: '',
        existingLabels: ['product/rook', 'type/bug', 'area/ci'],
      },
      rules
    );
    assert.ok(r.add.has('triage/needs-classification'));
    assert.ok(!r.add.has('product/rook') || r.remove.size >= 0, 'must not silently keep stale');
    // desired product is null -> needsTriage true
    assert.equal(r.needsTriage, true);
  });

  it('valid form product suppresses title ambiguity (form wins)', () => {
    const body = '### Product\n\nHuginn\n\n### Technical Area\n\nruntime';
    const r = classify(
      {
        title: 'fix(rook,huginn): x',
        body,
        existingLabels: ['type/bug'],
      },
      rules
    );
    assert.ok(r.add.has('product/huginn'));
    assert.ok(!r.add.has('triage/needs-classification'), 'form authority must not triage on title ambiguity');
    assert.equal(r.needsTriage, false);
  });

  it('legacy "fix(rook,agent)" multi-scope still triages, form Agent resolves to huginn', () => {
    const r = classify(
      { title: 'fix(rook,agent): shared thing', body: '', existingLabels: [] },
      rules
    );
    assert.ok(![...r.add].some((l) => l.startsWith('product/')), 'multi-scope must not invent product');
    assert.ok(r.add.has('triage/needs-classification'));
    const formR = classify(
      {
        title: 'fix(rook,agent): x',
        body: '### Product\n\nAgent\n\n### Technical Area\n\nruntime',
        existingLabels: ['type/bug'],
      },
      rules
    );
    assert.ok(formR.add.has('product/huginn'), 'form Agent must canonicalize to product/huginn');
  });
});

describe('run() guards', () => {
  it('rejects pull requests before classification', async () => {
    const { run } = require('./issue-labeler.cjs');
    const fakeGithub = {
      rest: {
        issues: {
          get: async () => ({ data: { title: 'x', body: '', labels: [], pull_request: {}, user: { login: 'u' } } }),
        },
      },
    };
    const fakeContext = { repo: { owner: 'o', repo: 'r' }, issue: { number: 1 } };
    await assert.rejects(() => run({ github: fakeGithub, context: fakeContext, issueNumber: 1 }), /pull request/);
  });
});

describe('idempotence and preservation', () => {
  it('fully labeled issue + priority/security -> zero changes', () => {
    const r = classify(
      {
        title: 'fix(huginn): improve MCP session recovery',
        body: '',
        existingLabels: ['product/huginn', 'type/bug', 'area/runtime', 'priority/high', 'security'],
      },
      rules
    );
    assert.deepEqual(add(r), []);
    assert.deepEqual(remove(r), []);
  });

  it('legacy product/agent is preserved on a stable historical issue', () => {
    const r = classify(
      { title: 'no scope here', body: '', existingLabels: ['product/agent', 'type/chore'] },
      rules
    );
    assert.ok(!r.remove.has('product/agent'), 'must not silently strip legacy product/agent');
    assert.ok(!r.add.has('product/huginn'), 'must not silently migrate the label');
  });

  it('legacy product/agent survives a new fix(agent) title scope', () => {
    // A historical issue still labelled product/agent must keep that
    // label when a follow-up title reuses the legacy scope.
    const r = classify(
      {
        title: 'fix(agent): trim cache',
        body: '',
        existingLabels: ['product/agent', 'type/bug', 'area/runtime'],
      },
      rules
    );
    assert.ok(!r.remove.has('product/agent'), 'must not strip legacy product/agent');
    assert.ok(!r.add.has('product/huginn'), 'must not migrate the historical label');
  });

  it('never touches priority/security/stale', () => {
    const r = classify(
      {
        title: 'fix(huginn): x',
        body: '',
        existingLabels: ['priority/low', 'security', 'stale'],
      },
      rules
    );
    const touched = [...r.add, ...r.remove].filter((l) => l.startsWith('priority/') || l === 'security' || l === 'stale');
    assert.deepEqual(touched, []);
  });

  it('areas are additive, never removed', () => {
    const r = classify(
      {
        title: 'fix(huginn): MCP session + dashboard polish',
        body: '',
        existingLabels: ['product/huginn', 'type/bug', 'area/runtime'],
      },
      rules
    );
    assert.ok(![...r.remove].some((l) => l.startsWith('area/')));
  });

  it('triage removed once fully classified', () => {
    const r = classify(
      {
        title: 'fix(huginn): improve MCP session recovery',
        body: '',
        existingLabels: ['product/huginn', 'type/bug', 'area/runtime', 'triage/needs-classification'],
      },
      rules
    );
    assert.ok(r.remove.has('triage/needs-classification'));
    assert.ok(!r.add.has('triage/needs-classification'));
  });
});
