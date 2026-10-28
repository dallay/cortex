'use strict';

const { it } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { evaluateRequiredChecks } = require('./ci-required.cjs');

const workflow = fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'), 'utf8');
const categories = ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs', 'labeler'];

function jobBlock(id) {
  const block = workflow.match(new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [\\w-]+:|$(?![\\s\\S]))`, 'm'));
  assert.ok(block, `job ${id}`);
  return block[1];
}

function evaluateBooleanExpression(expression) {
  const tokens = [];
  const tokenPattern = /\s*(\|\||&&|==|[()]|true|false|"[^"\\]*(?:\\.[^"\\]*)*"|'[^']*')/gy;
  let offset = 0;
  while (offset < expression.length) {
    tokenPattern.lastIndex = offset;
    const match = tokenPattern.exec(expression);
    assert.ok(match, `token inesperado en expresión: ${expression.slice(offset)}`);
    tokens.push(match[1]);
    offset = tokenPattern.lastIndex;
  }

  let position = 0;
  const peek = () => tokens[position];
  const consume = (expected) => {
    assert.equal(tokens[position], expected);
    position += 1;
  };
  const literal = () => {
    const token = tokens[position++];
    assert.ok(token !== undefined, 'literal esperado');
    if (token === 'true') return true;
    if (token === 'false') return false;
    assert.match(token, /^(?:"(?:[^"\\]|\\.)*"|'[^']*')$/);
    return token[0] === '"' ? JSON.parse(token) : token.slice(1, -1);
  };
  const atom = () => {
    if (peek() === '(') {
      consume('(');
      const value = disjunction();
      consume(')');
      return value;
    }
    const left = literal();
    if (peek() === '==') {
      consume('==');
      return left === literal();
    }
    assert.equal(typeof left, 'boolean', 'se esperaba una comparación');
    return left;
  };
  const conjunction = () => {
    let value = atom();
    while (peek() === '&&') {
      consume('&&');
      const right = atom();
      value = value && right;
    }
    return value;
  };
  const disjunction = () => {
    let value = conjunction();
    while (peek() === '||') {
      consume('||');
      const right = conjunction();
      value = value || right;
    }
    return value;
  };

  const result = disjunction();
  assert.equal(position, tokens.length, 'tokens restantes');
  return result;
}

function condition(id, selected, eventName = 'pull_request') {
  const block = jobBlock(id);
  let expression = block.match(/^    if: (.+)$/m)?.[1];
  if (expression === '|') expression = block.match(/^    if: \|\n((?:      .+\n)+)/m)?.[1];
  assert.ok(expression, `condición de ${id}`);
  const resolved = expression.trim()
    .replaceAll('github.event_name', JSON.stringify(eventName))
    .replace(/needs\.changes\.outputs\.(\w+)/g, (_, key) => JSON.stringify(selected[key]))
    .replaceAll('always()', 'true')
    .replace(/needs\.[\w-]+\.result/g, '"success"');
  return evaluateBooleanExpression(resolved);
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
  ['.github/scripts/check-semgrep-path-exceptions.py', ['github']],
  ['.github/scripts/test-semgrep-path-exceptions.py', ['github']],
  ['.github/scripts/exec-args.py', ['backend', 'github']],
  ['.github/scripts/make-shell', ['backend', 'github']],
  ['Makefile', ['backend', 'github']],
  ['scripts/setup-codegraph.sh', ['backend', 'github']],
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

it('valida SONAR_TOKEN antes de ejecutar acciones de la PR y no tiene excepciones', () => {
  const sonar = jobBlock('sonar');
  const validation = sonar.match(/^        run: \|\n((?:          .*\n)+)/m)?.[1];
  assert.ok(validation);
  assert.ok(sonar.indexOf('run: |') < sonar.indexOf('uses:'), 'validación anterior a acciones y código de la PR');
  assert.match(validation, /if \[ -z "\$SONAR_TOKEN" \]; then/);
  assert.match(validation, /exit 1/);
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
  it(`Gitleaks configura el rango completo en ${eventName} sin operaciones remotas`, () => {
    const scan = jobBlock('gitleaks-pr').match(/^      - name: Analizar historial de commits\n([\s\S]*?)(?=^      - )/m)?.[1];
    const script = scan?.match(/^        run: \|\n((?:          .*\n)+)/m)?.[1];
    assert.ok(script);
    assert.doesNotMatch(script, /\bgit (?:fetch|push|pull)\b|curl|wget/);
    assert.match(script, /^\s*LOG_OPTS="--all"\s*$/m);
    assert.match(script, /if \[ "\$GITHUB_EVENT_NAME" = "pull_request" \]; then/);
    assert.match(script, /LOG_OPTS="\$BASE_SHA\.\.\$HEAD_SHA"/);
    assert.match(script, /--log-opts="\$LOG_OPTS"/);
    assert.match(script, /--config \.gitleaks\.toml/);
    assert.match(script, /--report-format sarif/);
    assert.match(script, /--exit-code 1/);
  });
}

it('falla solo ante findings Semgrep activos, no ante excepciones SARIF en source', () => {
  const scan = jobBlock('semgrep-pr');
  assert.match(scan, /FINDINGS_COUNT=.*suppressions/);
  assert.match(scan, /select\(\(\.suppressions \/\/ \[\]\) \| length == 0\)/);
});

it('autentica los controles y el escaneo Semgrep con el secret de CI', () => {
  const scan = jobBlock('semgrep-pr');
  for (const name of ['Check exact path exceptions and scanner controls', 'Run Semgrep SAST scan']) {
    const step = scan.match(new RegExp(`^      - name: ${name}\\n([\\s\\S]*?)(?=^      - )`, 'm'))?.[1];
    assert.ok(step, name);
    assert.match(step, /SEMGREP_APP_TOKEN: \$\{\{ secrets\.SEMGREP_APP_TOKEN \}\}/);
  }
});

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
