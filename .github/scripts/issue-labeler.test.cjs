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
  it('fix(agent): MCP session recovery -> agent/bug/runtime, no triage', () => {
    const r = classify({ title: 'fix(agent): improve MCP session recovery', body: '', existingLabels: [] }, rules);
    assert.deepEqual(add(r), ['area/runtime', 'product/agent', 'type/bug']);
    assert.deepEqual(remove(r), []);
    assert.equal(r.needsTriage, false);
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

  it('test(agent): maps to type/test', () => {
    const r = classify({ title: 'test(agent): session replay harness', body: '', existingLabels: [] }, rules);
    assert.ok(r.add.has('type/test'));
    assert.ok(r.add.has('product/agent'));
  });

  it('multi-scope fix(rook,agent): is ambiguous -> triage, no invented product', () => {
    const t = parseConventionalTitle('fix(rook,agent): shared thing', rules);
    assert.equal(t.scope, null);
    const r = classify({ title: 'fix(rook,agent): shared thing', body: '', existingLabels: [] }, rules);
    assert.ok(![...r.add].some((l) => l.startsWith('product/')), 'must not invent product');
    assert.ok(r.add.has('triage/needs-classification'));
  });
});

describe('issue forms (highest priority)', () => {
  it('form Rook/providers wins over title scope', () => {
    const body = '### Product\n\nRook\n\n### Technical Area\n\nproviders\n\n### Description\n\ntimeout';
    const r = classify({ title: 'fix(agent): gateway timeout', body, existingLabels: ['type/bug'] }, rules);
    assert.ok(r.add.has('product/rook'), 'form product wins');
    assert.ok(r.add.has('area/providers'));
    // type preserved from existing (form frontmatter), not overwritten by title
    assert.ok(!r.add.has('type/bug'), 'type already present -> no add');
  });

  it('invalid form product -> triage, no fallback guessing', () => {
    const body = '### Product\n\nUnknownProduct\n\n### Technical Area\n\nruntime';
    const r = classify({ title: 'fix(agent): x', body, existingLabels: [] }, rules);
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
    assert.ok(!r.add.has('product/agent'));
    assert.deepEqual(remove(r).filter((l) => l.startsWith('product/')), []);
  });

  it('explicit form replaces conflicting product (safe correction)', () => {
    const body = '### Product\n\nAgent\n\n### Technical Area\n\nruntime';
    const r = classify(
      { title: 'fix(rook): migrate to agent runtime', body, existingLabels: ['product/rook', 'type/bug'] },
      rules
    );
    assert.ok(r.add.has('product/agent'));
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
      { title: 'fix(agent): old prefix', body: '', existingLabels: ['product/agent', 'type/feature', 'area/runtime'] },
      rules
    );
    assert.ok(!r.add.has('type/bug'), 'must not fight human type');
    assert.ok(!r.remove.has('type/feature'));
  });
});

describe('renovate and bots', () => {
  it('Dependency Dashboard -> shared/dependencies/chore', () => {
    const r = classify({ title: 'Dependency Dashboard', body: '', existingLabels: [] }, rules);
    assert.ok(r.add.has('product/shared'));
    assert.ok(r.add.has('area/dependencies'));
    assert.ok(r.add.has('type/chore'));
  });
});

describe('idempotence and preservation', () => {
  it('fully labeled issue + priority/security -> zero changes', () => {
    const r = classify(
      {
        title: 'fix(agent): improve MCP session recovery',
        body: '',
        existingLabels: ['product/agent', 'type/bug', 'area/runtime', 'priority/high', 'security'],
      },
      rules
    );
    assert.deepEqual(add(r), []);
    assert.deepEqual(remove(r), []);
  });

  it('never touches priority/security/stale', () => {
    const r = classify(
      {
        title: 'fix(agent): x',
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
        title: 'fix(agent): MCP session + dashboard polish',
        body: '',
        existingLabels: ['product/agent', 'type/bug', 'area/runtime'],
      },
      rules
    );
    assert.ok(![...r.remove].some((l) => l.startsWith('area/')));
  });

  it('triage removed once fully classified', () => {
    const r = classify(
      {
        title: 'fix(agent): improve MCP session recovery',
        body: '',
        existingLabels: ['product/agent', 'type/bug', 'area/runtime', 'triage/needs-classification'],
      },
      rules
    );
    assert.ok(r.remove.has('triage/needs-classification'));
    assert.ok(!r.add.has('triage/needs-classification'));
  });
});
