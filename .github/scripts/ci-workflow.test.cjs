'use strict';

const { it } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { evaluateRequiredChecks } = require('./ci-required.cjs');

const workflow = fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'), 'utf8');
const categories = ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs', 'labeler'];

function jobBlock(id) {
  const block = workflow.match(new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [\\w-]+:|$(?![\\s\\S]))`, 'm'));
  assert.ok(block, `job ${id}`);
  return block[1];
}

function condition(id, selected, eventName = 'pull_request') {
  const block = jobBlock(id);
  let expression = block.match(/^    if: (.+)$/m)?.[1];
  if (expression === '|') expression = block.match(/^    if: \|\n((?:      .+\n)+)/m)?.[1];
  assert.ok(expression, `condición de ${id}`);
  const resolved = expression
    .replaceAll('github.event_name', JSON.stringify(eventName))
    .replace(/needs\.changes\.outputs\.(\w+)/g, (_, key) => JSON.stringify(selected[key]))
    .replaceAll('always()', 'true')
    .replace(/needs\.[\w-]+\.result/g, '"success"');
  // Solo se evalúan comparaciones de categorías y eventos, nunca código del workflow.
  assert.match(resolved, /^[\s\w'"=|&()-]+$/);
  return Function(`"use strict"; return (${resolved});`)();
}

const routeCases = [
  ['pnpm-lock.yaml', ['frontend']],
  ['pnpm-workspace.yaml', ['frontend']],
  ['package.json', ['frontend', 'backend', 'docker', 'doctor']],
  ['justfile', ['frontend', 'backend', 'docker', 'doctor']],
  ['apps/rook/build.rs', ['backend']],
  ['apps/rook/tests/api.rs', ['backend']],
  ['apps/rook/examples/demo.rs', ['backend']],
  ['apps/rook/benches/router.rs', ['backend']],
  ['apps/rook/nuevo-directorio/datos.bin', ['backend']],
  ['.github/scripts/check-huginn-architecture.py', ['backend']],
  ['.dockerignore', ['docker']],
  ['.semgrepignore', ['github']],
  ['dev/test-configs/rook.toml', ['docker']],
  ['.github/scripts/ci-quality-ruleset.cjs', ['github']],
  ['.github/scripts/ci-quality-ruleset.test.cjs', ['github']],
  ['.github/scripts/ci-workflow.test.cjs', ['github']],
  ['.github/ci-path-filters.json', ['github']],
  ['.github/ci-quality-policy.json', ['github']],
  // El paquete completo incluye el dashboard: se aceptan controles backend adicionales.
  ['apps/rook/dashboard/src/main.tsx', ['frontend', 'backend']],
  ['apps/huginn/src/main.rs', ['backend']],
  ['crates/rook-core/src/lib.rs', ['backend']],
  ['Dockerfile.dev', ['docker']],
  ['apps/rook/Dockerfile', ['backend', 'docker']],
  ['dev/Dockerfile.runtime', ['docker']],
  ['nuevo/directorio/Dockerfile.ci', ['docker']],
  ['dev/e2e/run-api-keys-e2e.sh', ['docker']],
  ['apps/rook/dashboard/e2e/login.spec.ts', ['frontend', 'docker']],
  ['docs/development.md', ['docs']],
];

it('cubre todo el paquete Rook sin enumerar directorios', () => {
  const filters = JSON.parse(fs.readFileSync(path.join(__dirname, '../ci-path-filters.json'), 'utf8'));
  // Dorny usa picomatch con dot:true; este patrón también cubre rutas ocultas.
  // No se usa matchesGlob para rutas ocultas porque Node aplica otra semántica.
  assert.ok(filters.backend.includes('apps/rook/**'));
});

for (const [file, expected] of routeCases) {
  it(`enruta ${file} a sus controles`, () => {
    const filters = JSON.parse(fs.readFileSync(path.join(__dirname, '../ci-path-filters.json'), 'utf8'));
    assert.deepEqual(Object.keys(filters).sort(), [...categories].sort());
    for (const category of expected) {
      assert.ok(filters[category].some((pattern) => path.matchesGlob(file, pattern)), `${file} → ${category}`);
    }
    const selected = Object.fromEntries(categories.map((key) => [key, String(filters[key].some((pattern) => path.matchesGlob(file, pattern)))]));
    if (file === 'docs/development.md') {
      for (const key of ['backend', 'frontend', 'docker', 'github']) assert.equal(selected[key], 'false');
    }
    for (const eventName of ['push', 'pull_request']) {
      for (const id of ['doctor', 'build-dashboard', 'fmt', 'markdown', 'test-labeler', 'clippy', 'check', 'test', 'test-frontend', 'coverage', 'coverage-frontend', 'test-e2e', 'doc', 'audit', 'trivy-fs', 'semgrep-pr', 'sonar']) {
        const applicable = condition(id, selected, eventName);
        const outcome = evaluateRequiredChecks({
          eventName, sonarTokenPresent: true, changes: selected, results: { [id]: 'skipped' },
        });
        assert.equal(outcome.errors.some((error) => error === `${id}: expected success, got skipped`), applicable, `${eventName}: filtros → ${id} → Required`);
      }
    }
    if (selected.frontend === 'true') assert.equal(condition('trivy-fs', selected), true);
  });
}

it('usa los filtros probados en el detector', () => {
  assert.match(jobBlock('changes'), /filters:\s*\.github\/ci-path-filters\.json\s*$/m);
  for (const category of categories) {
    assert.ok(jobBlock('changes').includes(`${category}: \${{ steps.filter.outputs.${category} }}`));
    assert.ok(jobBlock('required').includes(`CHANGES_${category.toUpperCase()}: \${{ needs.changes.outputs.${category} }}`));
  }
});

it('ejecuta Sonar y su entrada dashboard en PR de documentación o metadatos', () => {
  for (const selected of [Object.fromEntries(categories.map((key) => [key, 'false'])), Object.fromEntries(categories.map((key) => [key, key === 'docs' ? 'true' : 'false']))]) {
    for (const id of ['sonar', 'build-dashboard']) {
      assert.equal(condition(id, selected), true, `${id} en PR`);
      assert.equal(condition(id, selected, 'push'), false, `${id} en push sin código`);
    }
  }
});

it('rechaza token ausente antes de checkout y setup, también en forks y Dependabot', () => {
  const sonar = jobBlock('sonar');
  const policy = sonar.match(/^        run: \|\n((?:          .*\n)+)/m)?.[1];
  assert.ok(policy);
  assert.ok(sonar.indexOf('run: |') < sonar.indexOf('uses:'), 'validación anterior a acciones y código de la PR');
  for (const actor of ['developer', 'dependabot[bot]']) {
    for (const fork of ['true', 'false']) {
      const child = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', policy], {
        encoding: 'utf8', env: { PATH: process.env.PATH, SONAR_TOKEN: '', GITHUB_EVENT_NAME: 'pull_request', GITHUB_ACTOR: actor, IS_FORK: fork },
      });
      assert.equal(child.status, 1, `${actor}, fork=${fork}: ${child.stdout} ${child.stderr}`);
    }
  }
  const available = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', policy], {
    encoding: 'utf8', env: { PATH: process.env.PATH, SONAR_TOKEN: 'credencial-ficticia' },
  });
  assert.equal(available.status, 0, available.stderr);
  assert.doesNotMatch(available.stdout + available.stderr, /credencial-ficticia/);
  assert.doesNotMatch(sonar, /sonar_exempt|continue-on-error:/);
});

it('espera el quality gate con 300 segundos y conserva ambas coberturas', () => {
  const sonar = jobBlock('sonar');
  assert.match(sonar, /^    timeout-minutes: 60\s*$/m);
  assert.match(sonar, /-Dsonar\.qualitygate\.wait=true/);
  assert.match(sonar, /-Dsonar\.qualitygate\.timeout=300/);
  assert.match(sonar, /pnpm exec vitest run --coverage/);
  assert.match(sonar, /cargo llvm-cov --workspace --all-features --lcov --output-path coverage\/rust-lcov\.info/);
  const scan = sonar.slice(sonar.indexOf('      - name: SonarQube Scan'));
  assert.doesNotMatch(scan, /^        if:/m);
  assert.doesNotMatch(workflow, /pull_request_target/);
  assert.match(sonar, /permissions:\n      contents: read\n    steps:/);
});

it('Gitleaks dispone del historial completo sin persistir credenciales', () => {
  const gitleaks = jobBlock('gitleaks-pr');
  assert.match(gitleaks, /fetch-depth: 0\s*\n\s*persist-credentials: false/);
  assert.match(gitleaks, /BASE_SHA: \$\{\{ github\.event\.pull_request\.base\.sha \}\}/);
  assert.match(gitleaks, /HEAD_SHA: \$\{\{ github\.event\.pull_request\.head\.sha \|\| github\.sha \}\}/);
  assert.doesNotMatch(gitleaks, /--all -n 50|fetch-depth: 50/);
});

for (const eventName of ['pull_request', 'push']) {
  it(`Gitleaks analiza el rango completo en ${eventName} sin operaciones remotas`, () => {
    const scan = jobBlock('gitleaks-pr').match(/^      - name: Analizar historial de commits\n([\s\S]*?)(?=^      - )/m)?.[1];
    const script = scan?.match(/^        run: \|\n((?:          .*\n)+)/m)?.[1];
    assert.ok(script);
    assert.doesNotMatch(script, /\bgit (?:fetch|push|pull)\b|curl|wget/);
    const base = 'a'.repeat(40);
    const head = 'b'.repeat(40);
    const child = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', `mkdir() { :; }\ngitleaks() { printf '[%s]\\n' "$@"; }\n${script}`], {
      encoding: 'utf8', env: { PATH: process.env.PATH, GITHUB_EVENT_NAME: eventName, BASE_SHA: base, HEAD_SHA: head },
    });
    assert.equal(child.status, 0, child.stderr);
    assert.ok(child.stdout.includes(`[--log-opts=${eventName === 'pull_request' ? `${base}..${head}` : '--all'}]`), child.stdout);
    for (const flag of ['--config', '.gitleaks.toml', '--report-format', 'sarif', '--exit-code', '1']) assert.ok(child.stdout.includes(`[${flag}]`));
  });
}

it('ejecuta siempre las pruebas CI y exige su resultado real', () => {
  const contract = jobBlock('ci-contract');
  assert.match(contract, /^    if: always\(\)\s*$/m);
  assert.doesNotMatch(contract, /^    needs:/m);
  assert.match(contract, /run: node --test \.github\/scripts\/ci-\*\.test\.cjs\s*$/m);
  assert.doesNotMatch(contract, /continue-on-error:/);
  assert.match(jobBlock('required'), /^      - ci-contract\s*$/m);
  assert.match(jobBlock('required'), /RESULT_CI_CONTRACT: \$\{\{ needs\.ci-contract\.result \}\}/);
  assert.doesNotMatch(jobBlock('required'), /continue-on-error:/);
});

it('conecta cada dependencia de Required con el runner sin perder resultados', () => {
  const block = jobBlock('required');
  const needs = block.match(/^    needs:\n((?:      - [\w-]+\n)+)/m)?.[1];
  assert.ok(needs);
  const ids = [...needs.matchAll(/- ([\w-]+)/g)].map((match) => match[1]);
  const runner = fs.readFileSync(path.join(__dirname, 'ci-required-run.cjs'), 'utf8');
  for (const id of ids) {
    const key = id.replaceAll('-', '_').toUpperCase();
    assert.ok(block.includes(`RESULT_${key}: \${{ needs.${id}.result }}`), id);
    assert.ok(runner.includes(`'${key}'`), id);
  }
});
