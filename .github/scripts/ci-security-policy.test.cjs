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
  assert.match(security, /no guaranteed acknowledgment, fix, or disclosure SLA/i);
  assert.doesNotMatch(security, /(?:>=\s*2\.x|1\.x).*:white_check_mark:/);

  for (const product of ['Rook', 'Huginn']) {
    assert.ok(security.includes(`| ${product} | No published GitHub Release |`),
        `${product} must have its own honest support row`);
    const version = manifest[`apps/${product.toLowerCase()}`];
    assert.ok(security.includes(`tracks \`${version}\` for`) ||
        security.includes(`tracks \`${version}\` for\n`),
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
  assert.match(security, /administrator must confirm/);
  assert.ok(!ci.includes('run: pnpm audit'));
  assert.match(security, /does \*\*not\*\* define a dedicated \`pnpm audit\` merge gate/);
});
