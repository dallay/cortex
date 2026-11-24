'use strict';

const {it} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '../..');

function read(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), 'utf8');
}

const security = read('SECURITY.md');

it('describes both product versions as pre-GA without inventing support', () => {
  const manifest = JSON.parse(read('.release-please-manifest.json'));
  assert.deepEqual(Object.keys(manifest).sort(), ['apps/huginn', 'apps/rook']);
  assert.match(security, /Current pre-GA position/);
  assert.match(security, /No supported released version/);
  assert.match(security, /no guaranteed acknowledgment, fix, or disclosure\s+SLA/i);
  assert.doesNotMatch(security, /(?:>=\s*2\.x|1\.x).*:white_check_mark:/);

  for (const product of ['Rook', 'Huginn']) {
    assert.ok(security.includes(`| ${product} | No published GitHub Release |`),
        `${product} must have its own honest support row`);
    const version = manifest[`apps/${product.toLowerCase()}`];
    assert.ok(security.includes(`tracks \`${version}\` for`),
    'the support policy must describe the current manifest version');
  }
});

it('links the policy from the product documentation entry points', () => {
  const productDocs = [
    'apps/rook/README.md',
    'docs/rook/README.md',
    'docs/huginn/README.md',
  ];
  for (const docPath of productDocs) {
    const link = '../../SECURITY.md';
    assert.ok(read(docPath).includes(`](${link})`),
        `${docPath} must link to the repository-wide policy`);
    assert.ok(fs.existsSync(path.resolve(repoRoot, path.dirname(docPath),
        link)), `${docPath}: policy link must resolve`);
  }
  assert.ok(read('docs/README.md').includes('](../SECURITY.md)'));
});

it('describes actual security jobs and does not claim a pnpm audit gate', () => {
  const ci = read('.github/workflows/ci.yml');
  const deep = read('.github/workflows/security-deep.yml');
  const gitguardian = read('.github/workflows/gitguardian-scan.yml');
  for (const job of ['audit', 'trivy-fs', 'gitleaks-pr', 'semgrep-pr']) {
    assert.ok(ci.includes(`  ${job}:\n`), `missing merge-gate job: ${job}`);
    assert.ok(security.includes(`\`${job}\``), `unlisted scan: ${job}`);
  }
  assert.match(deep, /cron: '17 3 \* \* 1-5'/);
  assert.match(deep, /reporting-only/i);
  assert.match(gitguardian, /github\.event\.pull_request\.head\.repo\.fork == false/);
  assert.match(security, /Private vulnerability reporting/);
  assert.match(security, /Private vulnerability reporting is \*\*enabled\*\*/);
  assert.match(security, /administrator must re-confirm it/);
  assert.ok(!ci.includes('run: pnpm audit'));
  assert.match(security, /does \*\*not\*\* define a dedicated \`pnpm audit\` merge gate/);
});

it('names a triage owner instead of promising an unnamed response', () => {
  assert.match(security, /### Triage Ownership/);
  assert.match(security, /owned by the repository maintainers/);
  assert.match(security, /`dallay`/);
  assert.match(security, /@yuniel-acosta/);
  assert.match(security, /no response-time commitment of any kind/i);
  assert.match(security, /no dedicated security team/i);
  // A process, not a slogan: the triage steps must be enumerated.
  const triage = security.slice(security.indexOf('### Triage Ownership'),
      security.indexOf('## Scope'));
  assert.match(triage, /1\..*2\..*3\./s);
  // No unbounded response deadline may reappear in the policy.
  assert.doesNotMatch(security, /within \d+ (?:business )?(?:hours|days)/i);
});

it('reports the documented cargo audit exception instead of hiding it', () => {
  const ci = read('.github/workflows/ci.yml');
  assert.match(ci, /cargo audit --ignore RUSTSEC-2026-0594/);
  assert.match(security, /RUSTSEC-2026-0594/);
  assert.match(security, /Blocking/);
});

it('separates blocking controls from reporting-only controls', () => {
  const ci = read('.github/workflows/ci.yml');
  const row = (name) => {
    const line = security.split('\n').find((l) => l.startsWith(`| ${name} `) ||
      l.startsWith(`| \`${name}\` `));
    assert.ok(line, `missing controls row for ${name}`);
    return line;
  };
  // Trivy publishes SARIF but never fails the job on findings.
  assert.match(ci, /exit-code: '0'/);
  assert.match(row('Trivy'), /Reporting-only/);
  // Semgrep and Gitleaks do fail on findings.
  assert.match(row('Semgrep'), /Blocking/);
  assert.match(row('Gitleaks'), /Blocking/);
  // The Sonar quality gate is not enforced in CI.
  assert.match(ci, /qualitygate\.wait removed/);
  assert.match(row('SonarCloud'), /quality gate does \*\*not\*\* block/);
});

it('documents the separate OSV-Scanner workflow and its schedule', () => {
  const osv = read('.github/workflows/osv-scanner.yml');
  assert.match(osv, /cron: "17 3 \* \* 2"/);
  assert.match(security, /osv-scanner\.yml/);
  assert.match(security, /OSV-Scanner/);
  assert.match(security, /not\*\* part of the `CI \/ Required` aggregate/);
});

it('keeps every repository link and anchor in the policy resolvable', () => {
  const slug = (heading) => heading.trim().toLowerCase()
      .replace(/`/g, '')
      .replace(/\[|\]/g, '')
      .replace(/[^\p{L}\p{N}\s_-]/gu, '')
      .replace(/\s+/g, '-');
  const anchorsOf = (markdown) => new Set(markdown.split('\n')
      .filter((line) => /^#{1,6}\s/.test(line))
      .map((line) => slug(line.replace(/^#{1,6}\s/, ''))));

  const links = [...security.matchAll(/\]\(([^)\s]+)\)/g)].map((m) => m[1]);
  assert.ok(links.length > 10, 'the policy must keep its reference links');

  for (const link of links) {
    if (/^(?:https?:|mailto:|#)/.test(link)) continue;
    const [target, anchor] = link.split('#');
    const resolved = path.resolve(repoRoot, target);
    assert.ok(fs.existsSync(resolved), `broken link target: ${link}`);
    if (!anchor) continue;
    const anchors = anchorsOf(fs.readFileSync(resolved, 'utf8'));
    assert.ok(anchors.has(anchor),
        `broken anchor: ${link} (heading slugs: ${[...anchors].join(', ')})`);
  }
});
