'use strict';

/**
 * Deterministic issue classifier for CORTEX.
 *
 * Priority (explicit, in order):
 *   01 Issue Form body (### Product / ### Technical Area)
 *   02 Conventional title `prefix(scope):` — scope gives product, prefix gives type
 *   03 Existing valid labels (preserve human decisions, never fight them)
 *   04 Triage — add triage/needs-classification, never invent a product.
 *
 * Invariants:
 *   - exactly one product/* (rook|agent|shared)
 *   - exactly one type/* when determinable, else triage
 *   - at least one area/* when determinable, else triage
 *   - product is NEVER inferred from body keywords (avoids "Agent integration with Rook" doubles)
 *   - only managed prefixes are touched: product/, type/, area/, triage/
 *     preserved: priority/*, security, stale, and anything unmanaged.
 *   - idempotent: second run with same inputs produces no add/remove.
 */

const fs = require('fs');
const path = require('path');

function loadRules(rulesPath) {
  const fallback = path.join(__dirname, '..', 'issue-labeler-rules.json');
  const raw = fs.readFileSync(rulesPath || fallback, 'utf8');
  return JSON.parse(raw);
}

function firstNonEmptyLine(text) {
  for (const line of String(text || '').split('\n')) {
    const t = line.trim().replace(/^[-*]\s+/, '').replace(/^["']|["']$/g, '').trim();
    if (t && t.toLowerCase() !== '_no response_' && t.toLowerCase() !== 'none') return t;
  }
  return '';
}

/**
 * Parse a `### Heading` section from an Issue Form rendered body.
 * Returns the raw section text (trimmed) or '' when absent.
 */
function parseFormSection(body, heading) {
  const lines = String(body || '').split('\n');
  const target = String(heading || '').trim().toLowerCase();
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^#{2,4}\s+(.+?)\s*$/);
    if (m && m[1].trim().toLowerCase() === target) {
      start = i + 1;
      break;
    }
  }
  if (start < 0) return '';
  const collected = [];
  for (let i = start; i < lines.length; i++) {
    if (/^#{2,4}\s+.+/.test(lines[i])) break;
    collected.push(lines[i]);
  }
  return collected.join('\n').trim();
}

function parseFormProduct(body, rules) {
  const section = parseFormSection(body, rules.form.productHeading);
  if (!section) return { present: false, value: null };
  const v = firstNonEmptyLine(section).toLowerCase();
  if (rules.products.includes(v)) return { present: true, value: `product/${v}` };
  return { present: true, value: null, invalid: firstNonEmptyLine(section) };
}

function parseFormArea(body, rules) {
  const section = parseFormSection(body, rules.form.areaHeading);
  if (!section) return { present: false, value: null };
  const v = firstNonEmptyLine(section).toLowerCase();
  if (rules.areas.includes(v)) return { present: true, value: `area/${v}` };
  return { present: true, value: null, invalid: firstNonEmptyLine(section) };
}

function parseConventionalTitle(title, rules) {
  const m = String(title || '').match(/^\s*([A-Za-z]+)(?:\(([^)]+)\))?(!)?:/);
  if (!m) return { prefix: null, scope: null, type: null, scopeExplicit: false, scopeValid: false };
  const prefix = m[1].toLowerCase();
  const hasParens = m[2] !== undefined;
  const rawScope = (m[2] || '').trim().toLowerCase();
  let scope = null;
  let scopeExplicit = false;
  let scopeValid = false;
  if (hasParens) {
    scopeExplicit = true;
    const parts = rawScope.split(/[,\s/]+/).map((s) => s.trim()).filter(Boolean);
    const valid = parts.filter((p) => rules.scopes.includes(p));
    // Exactly one valid scope counts as explicit; multiple/unknown/empty => explicit but invalid.
    if (parts.length === 1 && valid.length === 1) {
      scope = valid[0];
      scopeValid = true;
    } else {
      scope = null;
      scopeValid = false;
    }
  }
  const type = rules.conventional[prefix] || null;
  return { prefix, scope, type, scopeExplicit, scopeValid };
}

function isWordChar(ch) {
  return !!ch && /[a-z0-9_]/.test(ch);
}

// Whole-token/phrase match without RegExp (avoids ReDoS flags; keywords are
// versioned config, haystack is user text). Boundary = start/end or non-word char.
function matchesWholeToken(hayLower, kwLower) {
  const needle = String(kwLower || '').trim().toLowerCase();
  if (!needle) return false;
  const hay = String(hayLower || '').toLowerCase();
  let from = 0;
  while (true) {
    const idx = hay.indexOf(needle, from);
    if (idx < 0) return false;
    const before = idx === 0 ? '' : hay[idx - 1];
    const after = idx + needle.length >= hay.length ? '' : hay[idx + needle.length];
    if (!isWordChar(before) && !isWordChar(after)) return true;
    from = idx + 1;
  }
}

function inferAreas(title, body, rules) {
  const hay = `${title || ''}\n${body || ''}`.toLowerCase();
  const found = new Set();
  for (const [area, keywords] of Object.entries(rules.areaKeywords || {})) {
    for (const raw of keywords) {
      const kw = String(raw || '').trim().toLowerCase();
      if (!kw) continue;
      // Whole-token match: avoids `auth` in `author`, `core` in `score`.
      // Multi-word keywords match as whole phrases with word boundaries.
      if (matchesWholeToken(hay, kw)) {
        found.add(area);
        break;
      }
    }
  }
  return found;
}

function splitExisting(existingLabels, rules) {
  const existing = (existingLabels || []).map(String);
  const productsAll = existing.filter((l) => l.startsWith('product/'));
  const typesAll = existing.filter((l) => l.startsWith('type/'));
  const areasAll = existing.filter((l) => l.startsWith('area/'));
  const triageAll = existing.filter((l) => l === rules.triageLabel);

  const validProducts = productsAll.filter((l) => rules.products.includes(l.slice('product/'.length)));
  const validTypes = typesAll.filter((l) => rules.types.includes(l.slice('type/'.length)));
  const validAreas = areasAll.filter((l) => rules.areas.includes(l.slice('area/'.length)));

  const invalidManaged = existing.filter((l) => {
    if (l.startsWith('product/')) return !rules.products.includes(l.slice(8));
    if (l.startsWith('type/')) return !rules.types.includes(l.slice(5));
    if (l.startsWith('area/')) return !rules.areas.includes(l.slice(5));
    if (l.startsWith('triage/')) return l !== rules.triageLabel;
    return false;
  });

  return { existing, productsAll, typesAll, areasAll, triageAll, validProducts, validTypes, validAreas, invalidManaged };
}

function isRenovateLike(title, rules) {
  const t = String(title || '').toLowerCase();
  return (rules.renovate?.titleContains || []).some((s) => t.includes(String(s).toLowerCase()));
}

function isAllowedRenovateActor(login, rules) {
  const allowed = (rules.renovate?.logins || []).map((s) => String(s).toLowerCase());
  return !!login && allowed.includes(String(login).toLowerCase());
}

function classify(input, rules) {
  const { title = '', body = '', existingLabels = [], author = null } = input || {};
  const split = splitExisting(existingLabels, rules);

  // Special case: Renovate bot maintenance -> shared + dependencies + chore.
  // Requires BOTH title match and allowlisted bot login so human issues
  // with similar titles do not trigger the exception.
  if (isRenovateLike(title, rules) && isAllowedRenovateActor(author, rules)) {
    const add = new Set();
    const remove = new Set();
    for (const l of [rules.renovate.product, rules.renovate.area, rules.renovate.type]) {
      if (l && !split.existing.includes(l)) add.add(l);
    }
    for (const p of split.productsAll) {
      if (p !== rules.renovate.product) remove.add(p);
    }
    for (const t of split.typesAll) {
      if (t !== rules.renovate.type) remove.add(t);
    }
    if (split.triageAll.length && split.invalidManaged.length === 0) {
      for (const t of split.triageAll) remove.add(t);
    }
    return { add, remove, reason: 'renovate-dashboard', desired: { product: rules.renovate.product } };
  }

  const formProduct = parseFormProduct(body, rules);
  const formArea = parseFormArea(body, rules);
  const conv = parseConventionalTitle(title, rules);
  const inferredAreas = inferAreas(title, body, rules);

  // --- Product: form > title scope > existing single --- never body keywords.
  // Explicit but invalid scope (e.g. fix(rook,agent):) must triage, never inherit stale product.
  let desiredProduct = null;
  let productSource = null;
  const explicitScopeInvalid = conv.scopeExplicit && !conv.scopeValid;
  if (formProduct.present && formProduct.value) {
    desiredProduct = formProduct.value;
    productSource = 'form';
  } else if (!formProduct.present && conv.scopeValid && conv.scope) {
    desiredProduct = `product/${conv.scope}`;
    productSource = 'title';
  } else if (!formProduct.present && !conv.scopeExplicit && split.validProducts.length === 1 && split.productsAll.length === 1) {
    desiredProduct = split.validProducts[0];
    productSource = 'existing';
  } else {
    desiredProduct = null;
    productSource = null;
  }
  const formProductInvalid = formProduct.present && !formProduct.value;

  // --- Type: preserve single existing; else title prefix; else triage.
  let desiredType = null;
  let typeSource = null;
  if (split.validTypes.length === 1 && split.typesAll.length === 1) {
    desiredType = split.validTypes[0];
    typeSource = 'existing';
  } else if (conv.type) {
    desiredType = conv.type;
    typeSource = 'title';
  }

  // --- Areas: form + existing valid + keyword inference (additive, never remove).
  const desiredAreas = new Set();
  if (formArea.present && formArea.value) desiredAreas.add(formArea.value);
  for (const a of split.validAreas) desiredAreas.add(a);
  for (const a of inferredAreas) desiredAreas.add(a);

  const needsTriage =
    !desiredProduct ||
    !desiredType ||
    desiredAreas.size === 0 ||
    formProductInvalid ||
    (explicitScopeInvalid && productSource !== 'form') ||
    (formArea.present && !formArea.value) ||
    split.invalidManaged.length > 0 ||
    (split.productsAll.length > 1 && productSource !== 'form' && productSource !== 'title');

  const add = new Set();
  const remove = new Set();

  if (desiredProduct) {
    if (!split.existing.includes(desiredProduct)) add.add(desiredProduct);
    if (productSource === 'form' || productSource === 'title') {
      for (const p of split.productsAll) {
        if (p !== desiredProduct) remove.add(p);
      }
    }
  }
  if (desiredType) {
    if (!split.existing.includes(desiredType)) add.add(desiredType);
    if (typeSource === 'title') {
      for (const t of split.typesAll) {
        if (t !== desiredType) remove.add(t);
      }
    }
  }
  for (const a of desiredAreas) {
    if (!split.existing.includes(a)) add.add(a);
  }

  if (needsTriage) {
    if (!split.existing.includes(rules.triageLabel)) add.add(rules.triageLabel);
  } else {
    for (const t of split.triageAll) remove.add(t);
  }

  const reason = [
    `product:${desiredProduct || 'triaged'}(${productSource || 'none'})`,
    `type:${desiredType || 'triaged'}(${typeSource || 'none'})`,
    `areas:${desiredAreas.size ? [...desiredAreas].join(',') : 'triaged'}`,
    needsTriage ? 'needs-triage' : 'classified',
  ].join(' ');

  return { add, remove, reason, desired: { product: desiredProduct, type: desiredType, areas: [...desiredAreas] }, needsTriage };
}

async function run({ github, context, issueNumber, rulesPath } = {}) {
  if (!github || !context) throw new Error('run({github, context}) is required (actions/github-script).');
  const owner = context.repo.owner;
  const repo = context.repo.repo;
  const num = Number(issueNumber || (context.issue && context.issue.number));
  if (!num) throw new Error('Missing issue number (event.issue.number or workflow_dispatch input).');
  const rules = loadRules(rulesPath);

  const { data: issue } = await github.rest.issues.get({ owner, repo, issue_number: num });
  if (issue.pull_request) {
    throw new Error(`Refusing to classify #${num}: it is a pull request (product exclusivity applies to issues only).`);
  }
  const title = issue.title || '';
  const ibody = issue.body || '';
  const existingLabels = (issue.labels || []).map((l) => (typeof l === 'string' ? l : l.name));
  const author = (issue.user && issue.user.login) || null;

  const result = classify({ title, body: ibody, existingLabels, author }, rules);

  const toAdd = [...result.add].filter((l) => !existingLabels.includes(l));
  const toRemove = [...result.remove].filter((l) => existingLabels.includes(l));

  if (toAdd.length > 0) {
    await github.rest.issues.addLabels({ owner, repo, issue_number: num, labels: toAdd });
  }
  for (const name of toRemove) {
    try {
      await github.rest.issues.removeLabel({ owner, repo, issue_number: num, name });
    } catch (err) {
      if (err && err.status !== 404) throw err;
    }
  }
  return { issue: num, toAdd, toRemove, reason: result.reason };
}

module.exports = {
  loadRules,
  parseFormSection,
  parseFormProduct,
  parseFormArea,
  parseConventionalTitle,
  inferAreas,
  classify,
  run,
};
