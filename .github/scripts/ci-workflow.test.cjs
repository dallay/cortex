'use strict';

const {it} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {evaluateRequiredChecks} = require('./ci-required.cjs');

const workflow = fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'),
    'utf8');
const categories = ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs',
  'labeler'];

function jobBlock(id) {
  const block = workflow.match(
      new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [\\w-]+:|$(?![\\s\\S]))`, 'm'));
  assert.ok(block, `job ${id}`);
  return block[1];
}

const gitleaksConfig = fs.readFileSync(
    path.join(__dirname, '../../.gitleaks.toml'), 'utf8');

function gitleaksAllowlistSections(ruleBody) {
  const sections = [];
  let current = null;
  for (const line of ruleBody.split('\n')) {
    if (/^\s*\[\[rules\.allowlists\]\]\s*$/.test(line)) {
      current = [];
      sections.push(current);
      continue;
    }
    if (current) {
      current.push(line);
    }
  }
  return sections.map((section) => section.join('\n'));
}

function gitleaksRules(config) {
  const globalMatch = config.match(/^\[allowlist\][\s\S]*$/m);
  const globalSection = globalMatch ? globalMatch[0] : '';
  const rulesText = globalMatch ? config.slice(0, globalMatch.index) : config;
  const bodies = rulesText.split(/^\[\[rules\]\]\s*$/m).slice(1);
  return {
    globalSection,
    rules: bodies.map((body) => ({
      id: body.match(/^\s*id = "([^"]+)"/m)?.[1],
      allowlists: gitleaksAllowlistSections(body),
    })),
  };
}

function evaluateBooleanExpression(expression) {
  const tokens = [];
  const tokenPattern = /\s*(\|\||&&|==|[()]|true|false|"[^"\\]*(?:\\.[^"\\]*)*"|'[^']*')/gy;
  let offset = 0;
  while (offset < expression.length) {
    tokenPattern.lastIndex = offset;
    const match = tokenPattern.exec(expression);
    assert.ok(match,
        `token inesperado en expresión: ${expression.slice(offset)}`);
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
    if (token === 'true') {
      return true;
    }
    if (token === 'false') {
      return false;
    }
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
  if (expression === '|') {
    expression = block.match(
        /^    if: \|\n((?:      .+\n)+)/m)?.[1];
  }
  assert.ok(expression, `condición de ${id}`);
  const resolved = expression.trim()
      .replaceAll('github.event_name', JSON.stringify(eventName))
      .replace(/needs\.changes\.outputs\.(\w+)/g,
          (_, key) => JSON.stringify(selected[key]))
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
  ['.config/nextest.toml', ['backend']],
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
  const filters = JSON.parse(
      fs.readFileSync(path.join(__dirname, '../ci-path-filters.json'), 'utf8'));
  // Dorny usa picomatch con dot:true; este patrón también cubre rutas ocultas.
  // No se usa matchesGlob para rutas ocultas porque Node aplica otra semántica.
  assert.ok(filters.backend.includes('apps/rook/**'));
});

for (const [file, expected] of routeCases) {
  it(`enruta ${file} a sus controles`, () => {
    const filters = JSON.parse(
        fs.readFileSync(path.join(__dirname, '../ci-path-filters.json'),
            'utf8'));
    assert.deepEqual(Object.keys(filters).sort(), [...categories].sort());
    for (const category of expected) {
      assert.ok(
          filters[category].some((pattern) => path.matchesGlob(file, pattern)),
          `${file} → ${category}`);
    }
    const selected = Object.fromEntries(categories.map((key) => [key, String(
        filters[key].some((pattern) => path.matchesGlob(file, pattern)))]));
    if (file === 'docs/development.md') {
      for (const key of
          ['backend', 'frontend', 'docker', 'github']) {
        assert.equal(
            selected[key], 'false');
      }
    }
    for (const eventName of ['push', 'pull_request']) {
      for (const id of
          ['doctor', 'build-dashboard', 'fmt', 'markdown', 'test-labeler',
            'clippy', 'check', 'test', 'test-frontend', 'coverage',
            'coverage-frontend', 'test-e2e', 'doc', 'audit', 'trivy-fs',
            'semgrep-pr', 'sonar']) {
        const applicable = condition(id, selected, eventName);
        const outcome = evaluateRequiredChecks({
          eventName,
          sonarTokenPresent: true,
          changes: selected,
          results: {[id]: 'skipped'},
        });
        assert.equal(outcome.errors.some(
                (error) => error === `${id}: expected success, got skipped`),
            applicable, `${eventName}: filtros → ${id} → Required`);
      }
    }
    if (selected.frontend === 'true') {
      assert.equal(
          condition('trivy-fs', selected), true);
    }
  });
}

it('usa los filtros probados en el detector', () => {
  assert.match(jobBlock('changes'),
      /filters:\s*\.github\/ci-path-filters\.json\s*$/m);
  for (const category of categories) {
    assert.ok(jobBlock('changes').includes(
        `${category}: \${{ steps.filter.outputs.${category} }}`));
    assert.ok(jobBlock('required').includes(
        `CHANGES_${category.toUpperCase()}: \${{ needs.changes.outputs.${category} }}`));
  }
});

it('ejecuta Sonar y su entrada dashboard en PR de documentación o metadatos',
    () => {
      for (const selected of
          [Object.fromEntries(categories.map((key) => [key, 'false'])),
            Object.fromEntries(categories.map(
                (key) => [key, key === 'docs' ? 'true' : 'false']))]) {
        for (const id of ['sonar', 'build-dashboard']) {
          assert.equal(condition(id, selected), true, `${id} en PR`);
          assert.equal(condition(id, selected, 'push'), false,
              `${id} en push sin código`);
        }
      }
    });

it('valida SONAR_TOKEN antes de ejecutar acciones de la PR y no tiene excepciones',
    () => {
      const sonar = jobBlock('sonar');
      const validation = sonar.match(
          /^        run: \|\n((?:          .*\n)+)/m)?.[1];
      assert.ok(validation);
      assert.ok(sonar.indexOf('run: |') < sonar.indexOf('uses:'),
          'validación anterior a acciones y código de la PR');
      assert.match(validation, /if \[ -z "\$SONAR_TOKEN" \]; then/);
      assert.match(validation, /exit 1/);
      assert.doesNotMatch(sonar, /sonar_exempt|continue-on-error:/);
    });

it('espera el quality gate con 300 segundos y conserva ambas coberturas',
    () => {
      const sonar = jobBlock('sonar');
      assert.match(sonar, /^    timeout-minutes: 60\s*$/m);
      assert.match(sonar, /-Dsonar\.qualitygate\.wait=true/);
      assert.match(sonar, /-Dsonar\.qualitygate\.timeout=300/);
      assert.match(sonar, /pnpm exec vitest run --coverage/);
      assert.match(sonar,
          /cargo llvm-cov --workspace --all-features --lcov --output-path coverage\/rust-lcov\.info/);
      const scan = sonar.slice(sonar.indexOf('      - name: SonarQube Scan'));
      assert.doesNotMatch(scan, /^        if:/m);
      assert.doesNotMatch(workflow, /pull_request_target/);
      assert.match(sonar, /permissions:\n      contents: read\n    steps:/);
    });

it('Gitleaks dispone del historial completo sin persistir credenciales', () => {
  const gitleaks = jobBlock('gitleaks-pr');
  assert.match(gitleaks, /fetch-depth: 0\s*\n\s*persist-credentials: false/);
  assert.match(gitleaks,
      /BASE_SHA: \$\{\{ github\.event\.pull_request\.base\.sha \}\}/);
  assert.match(gitleaks,
      /HEAD_SHA: \$\{\{ github\.event\.pull_request\.head\.sha \|\| github\.sha \}\}/);
  assert.doesNotMatch(gitleaks, /--all -n 50|fetch-depth: 50/);
});

for (const eventName of ['pull_request', 'push']) {
  it(`Gitleaks configura el rango completo en ${eventName} sin operaciones remotas`,
      () => {
        const scan = jobBlock('gitleaks-pr').match(
            /^      - name: Analizar historial de commits\n([\s\S]*?)(?=^      - )/m)?.[1];
        const script = scan?.match(
            /^        run: \|\n((?:          .*\n)+)/m)?.[1];
        assert.ok(script);
        assert.doesNotMatch(script, /\bgit (?:fetch|push|pull)\b|curl|wget/);
        assert.match(script, /^\s*LOG_OPTS="--all"\s*$/m);
        assert.match(script,
            /if \[ "\$GITHUB_EVENT_NAME" = "pull_request" \]; then/);
        assert.match(script, /LOG_OPTS="\$BASE_SHA\.\.\$HEAD_SHA"/);
        assert.match(script, /--log-opts="\$LOG_OPTS"/);
        assert.match(script, /--config \.gitleaks\.toml/);
        assert.match(script, /--report-format sarif/);
        assert.match(script, /--exit-code 1/);
      });
}

it('falla solo ante findings Semgrep activos, no ante excepciones SARIF en source',
    () => {
      const scan = jobBlock('semgrep-pr');
      assert.match(scan, /FINDINGS_COUNT=.*suppressions/);
      assert.match(scan,
          /select\(\(\.suppressions \/\/ \[\]\) \| length == 0\)/);
    });

it('autentica los controles y el escaneo Semgrep con el secret de CI', () => {
  const scan = jobBlock('semgrep-pr');
  for (const name of ['Check exact path exceptions and scanner controls',
    'Run Semgrep SAST scan']) {
    const step = scan.match(
        new RegExp(`^      - name: ${name}\\n([\\s\\S]*?)(?=^      - )`,
            'm'))?.[1];
    assert.ok(step, name);
    assert.match(step,
        /SEMGREP_APP_TOKEN: \$\{\{ secrets\.SEMGREP_APP_TOKEN \}\}/);
  }
});

it('ejecuta siempre las pruebas CI y exige su resultado real', () => {
  const contract = jobBlock('ci-contract');
  assert.match(contract, /^    if: always\(\)\s*$/m);
  assert.doesNotMatch(contract, /^    needs:/m);
  assert.match(contract,
      /run: node --test \.github\/scripts\/ci-\*\.test\.cjs\s*$/m);
  assert.doesNotMatch(contract, /continue-on-error:/);
  assert.match(jobBlock('required'), /^      - ci-contract\s*$/m);
  assert.match(jobBlock('required'),
      /RESULT_CI_CONTRACT: \$\{\{ needs\.ci-contract\.result \}\}/);
  assert.doesNotMatch(jobBlock('required'), /continue-on-error:/);
});

it('exige enforcement Seatbelt de Huginn en PR sobre macOS sin filtrar tests',
    () => {
      const seatbelt = jobBlock('test-huginn-seatbelt');
      assert.match(seatbelt, /^    runs-on: macos-latest\s*$/m);
      assert.match(seatbelt, /needs\.changes\.outputs\.backend == 'true'/);
      assert.match(seatbelt, /needs\.changes\.outputs\.github == 'true'/);
      assert.match(seatbelt, /test "\$RUNNER_OS" = "macOS"/);
      assert.match(seatbelt, /sw_vers/);
      assert.match(seatbelt, /test -x \/usr\/bin\/sandbox-exec/);
      assert.match(seatbelt,
          /cargo test -p huginn-runtime 2>&1 \| tee huginn-runtime-tests\.log/);
      for (const requiredTest of [
        'run_shell_can_read_and_write_inside_workspace',
        'run_shell_timeout_terminates_descendant_holding_output_pipes',
        'run_shell_cancellation_terminates_descendant_holding_output_pipes',
        'run_shell_cannot_read_external_marker_after_changing_cwd',
        'run_shell_cannot_read_marker_outside_workspace',
        'run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds',
        'run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds',
        'run_shell_seatbelt_denies_unix_socket_connect_after_host_control_succeeds',
      ]) {
        assert.ok(seatbelt.includes(requiredTest),
            `required test ${requiredTest}`);
      }
      assert.doesNotMatch(seatbelt, /--(?:skip|exact|ignored)|-- .*seatbelt/);
      assert.match(jobBlock('required'), /^      - test-huginn-seatbelt\s*$/m);
      assert.match(jobBlock('required'),
          /RESULT_TEST_HUGINN_SEATBELT: \$\{\{ needs\.test-huginn-seatbelt\.result \}\}/);
    });

it('acota la excepción de Gitleaks del fixture GM-05 a su archivo y su valor',
    () => {
      const {globalSection, rules} = gitleaksRules(gitleaksConfig);
      const generic = rules.find((rule) => rule.id === 'generic_secret');
      assert.ok(generic, 'regla generic_secret presente');

      const scoped = generic.allowlists.filter(
          (section) => section.includes('dev/muninn'));
      assert.equal(scoped.length, 1,
          'una sola allowlist de la regla menciona dev/muninn');
      const block = scoped[0];
      assert.ok(block.includes('condition = "AND"'),
          'condición AND para acotar la coincidencia');
      assert.ok(block.includes('regexTarget = "match"'),
          'el objetivo es la coincidencia completa');
      assert.ok(
          block.includes(
              `paths = ['''^dev/muninn/tests/test_gm05_build\\.py$''']`),
          'ruta exacta del test GM-05',
      );
      assert.ok(
          block.includes(
              `regexes = ['''(?i)^token\\s*=\\s*"synthetic-test-value$''']`),
          'valor exacto del token sintético',
      );
      assert.doesNotMatch(block, /condition = "OR"/);

      for (const rule of
          rules.filter((candidate) => candidate.id !== 'generic_secret')) {
        assert.ok(
            !rule.allowlists.some(
                (section) => section.includes('dev/muninn') || section.includes(
                    'synthetic-test-value')),
            `la regla ${rule.id} no declara excepciones del fixture GM-05`,
        );
      }

      assert.doesNotMatch(globalSection, /dev\/muninn|synthetic-test-value/,
          'sin allowlist global para el fixture');
    });

it('acota los marcadores sintéticos de Seatbelt a su archivo y sus valores', () => {
  const {globalSection, rules} = gitleaksRules(gitleaksConfig);
  const generic = rules.find((rule) => rule.id === 'generic_secret');
  assert.ok(generic, 'regla generic_secret presente');

  const scoped = generic.allowlists.filter(
      (section) => section.includes('crates/huginn/runtime/src/tools'));
  assert.equal(scoped.length, 1, 'una allowlist acotada para tools.rs');
  const block = scoped[0];
  assert.ok(block.includes('condition = "AND"'),
      'condición AND para acotar la coincidencia');
  assert.ok(block.includes('regexTarget = "match"'),
      'el objetivo es la coincidencia completa');
  assert.ok(
      block.includes(
          `paths = ['''^crates/huginn/runtime/src/tools\\.rs$''']`),
      'ruta exacta del módulo de herramientas',
  );
  assert.ok(
      block.includes(
          `'''(?i)^secret\\s*=\\s*"(other-workspace-secret-value|credential-fixture-secret-not-real|external-fixture-secret|unassigned-temp-fixture-secret|cwd-does-not-expand-permissions|symlink-target-fixture-secret)$'''`),
      'solo los marcadores sintéticos de las pruebas de denegación',
  );
  assert.doesNotMatch(block, /condition = "OR"/);

  for (const rule of
      rules.filter((candidate) => candidate.id !== 'generic_secret')) {
    assert.ok(
        !rule.allowlists.some(
            (section) => section.includes('crates/huginn/runtime/src/tools')),
        `la regla ${rule.id} no declara excepciones de Seatbelt`,
    );
  }

  assert.doesNotMatch(globalSection, /crates\/huginn\/runtime\/src\/tools\.rs/,
      'sin allowlist global para los fixtures de Seatbelt');
});

it('conecta cada dependencia de Required con el runner sin perder resultados',
    () => {
      const block = jobBlock('required');
      const needs = block.match(/^    needs:\n((?:      - [\w-]+\n)+)/m)?.[1];
      assert.ok(needs);
      const ids = [...needs.matchAll(/- ([\w-]+)/g)].map((match) => match[1]);
      const runner = fs.readFileSync(
          path.join(__dirname, 'ci-required-run.cjs'), 'utf8');
      for (const id of ids) {
        const key = id.replaceAll('-', '_').toUpperCase();
        assert.ok(block.includes(`RESULT_${key}: \${{ needs.${id}.result }}`),
            id);
        assert.ok(runner.includes(`'${key}'`), id);
      }
    });

function namedStep(id, name) {
  const step = jobBlock(id).match(
      new RegExp(
          `^      - name: ${name}\\n([\\s\\S]*?)(?=^      - |$(?![\\s\\S]))`,
          'm'));
  assert.ok(step, `${id}: step ${name}`);
  return step[1];
}

it('genera y sube resultados JUnit de Rust sin perder doctests', () => {
  const nextestConfigPath = path.join(__dirname, '../../.config/nextest.toml');
  assert.ok(fs.existsSync(nextestConfigPath), 'perfil CI de Nextest presente');
  const nextestConfig = fs.readFileSync(nextestConfigPath, 'utf8');
  assert.match(nextestConfig, /^\[profile\.ci\]$/m);
  assert.match(nextestConfig, /^\[profile\.ci\.junit\]$/m);
  assert.match(nextestConfig, /^path = "junit\.xml"$/m);

  const rust = jobBlock('test');
  assert.match(rust, /tool: nextest@0\.9\.151/);
  assert.match(rust,
      /cargo nextest run --workspace --all-features --profile ci/);
  assert.match(rust, /cargo test --workspace --all-features --doc/);

  const upload = namedStep('test', 'Upload Rust test results to Codecov');
  assert.match(upload, /if: \$\{\{ !cancelled\(\) &&/);
  assert.match(upload,
      /github\.event_name != 'pull_request' \|\| github\.event\.pull_request\.head\.repo\.full_name == github\.repository/);
  assert.match(upload,
      /if: \$\{\{ !cancelled\(\) && github\.actor != 'dependabot\[bot\]' &&/);
  assert.match(upload,
      /uses: codecov\/codecov-action@303a32d7a59b442fa8d48b6a1cc6825c09c847a5 # v7/);
  assert.match(upload, /report_type: test_results/);
  assert.match(upload, /files: target\/nextest\/ci\/junit\.xml/);
  assert.match(upload, /flags: backend/);
  assert.match(upload, /fail_ci_if_error: false/);
  assert.match(upload, /token: \$\{\{ secrets\.CODECOV_TOKEN \}\}/);

  const artifact = namedStep('test', 'Retain Rust JUnit report artifact');
  assert.match(artifact,
      /if: \$\{\{ !cancelled\(\) && hashFiles\('target\/nextest\/ci\/junit\.xml'\) != '' \}\}/);
  assert.match(artifact,
      /uses: actions\/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7\.0\.2/);
  assert.match(artifact, /path: target\/nextest\/ci\/junit\.xml/);
  assert.match(artifact, /retention-days: 7/);
  assert.match(artifact, /continue-on-error: true/);
});

it('genera y sube resultados JUnit de Vitest tras el job de frontend', () => {
  const frontend = jobBlock('test-frontend');
  assert.match(frontend,
      /pnpm exec vitest run --reporter=default --reporter=junit --outputFile\.junit=junit\.xml/);

  const upload = namedStep(
      'test-frontend', 'Upload Frontend test results to Codecov');
  assert.match(upload, /if: \$\{\{ !cancelled\(\) &&/);
  assert.match(upload,
      /github\.event_name != 'pull_request' \|\| github\.event\.pull_request\.head\.repo\.full_name == github\.repository/);
  assert.match(upload,
      /if: \$\{\{ !cancelled\(\) && github\.actor != 'dependabot\[bot\]' &&/);
  assert.match(upload,
      /uses: codecov\/codecov-action@303a32d7a59b442fa8d48b6a1cc6825c09c847a5 # v7/);
  assert.match(upload, /report_type: test_results/);
  assert.match(upload, /files: apps\/rook\/dashboard\/junit\.xml/);
  assert.match(upload, /flags: frontend/);
  assert.match(upload, /fail_ci_if_error: false/);
  assert.match(upload, /token: \$\{\{ secrets\.CODECOV_TOKEN \}\}/);

  const artifact = namedStep(
      'test-frontend', 'Retain Frontend JUnit report artifact');
  assert.match(artifact,
      /if: \$\{\{ !cancelled\(\) && hashFiles\('apps\/rook\/dashboard\/junit\.xml'\) != '' \}\}/);
  assert.match(artifact,
      /uses: actions\/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7\.0\.2/);
  assert.match(artifact, /path: apps\/rook\/dashboard\/junit\.xml/);
  assert.match(artifact, /retention-days: 7/);
  assert.match(artifact, /continue-on-error: true/);
});

it('activa Bundle Analysis solo para cambios frontend y limita el token', () => {
  const build = namedStep('build-dashboard', 'Build dashboard');
  assert.match(build,
      /CODECOV_BUNDLE_ANALYSIS: \$\{\{ needs\.changes\.outputs\.frontend == 'true' && github\.actor != 'dependabot\[bot\]' \}\}/);
  assert.match(build,
      /CODECOV_TOKEN: \$\{\{ needs\.changes\.outputs\.frontend == 'true' && github\.actor != 'dependabot\[bot\]' && \(github\.event_name != 'pull_request' \|\| github\.event\.pull_request\.head\.repo\.full_name == github\.repository\) && secrets\.CODECOV_TOKEN \|\| '' \}\}/);

  const viteConfig = fs.readFileSync(
      path.join(__dirname, '../../apps/rook/dashboard/vite.config.ts'), 'utf8');
  assert.match(viteConfig,
      /enableBundleAnalysis: process\.env\.CODECOV_BUNDLE_ANALYSIS === "true"/);
  assert.match(viteConfig,
      /uploadToken: process\.env\.CODECOV_TOKEN \|\| undefined/);
  assert.match(viteConfig, /gitService: "github"/);
});

it('publica pronto comentarios de cobertura y conserva los flags carryforward', () => {
  const codecovConfig = fs.readFileSync(
      path.join(__dirname, '../../codecov.yml'), 'utf8');
  assert.match(codecovConfig, /^  after_n_builds: 1$/m);
  assert.match(codecovConfig, /^  show_carryforward_flags: true$/m);
  assert.match(codecovConfig, /carryforward: true/);
  assert.match(codecovConfig, /warning_threshold: "5%"/);
  assert.match(codecovConfig, /status: "informational"/);
});

it('define componentes de cobertura por producto con estados informativos', () => {
  const codecovConfig = fs.readFileSync(
      path.join(__dirname, '../../codecov.yml'), 'utf8');

  assert.match(codecovConfig, /^component_management:$/m);

  // Los componentes se identifican por producto y no pisan los flags.
  for (const componentId of ['rook', 'huginn', 'dashboard']) {
    assert.match(codecovConfig,
        new RegExp(`^    - component_id: ${componentId}$`, 'm'));
  }

  // Rutas de cada producto, derivadas de la estructura del workspace.
  assert.match(codecovConfig, /^        - "apps\/rook\/src\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "apps\/rook\/tests\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "crates\/domain\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "crates\/application\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "crates\/infrastructure\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "apps\/huginn\/src\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "apps\/huginn\/tests\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "crates\/huginn\/\*\*"$/m);
  assert.match(codecovConfig, /^        - "apps\/rook\/dashboard\/\*\*"$/m);

  // Los estados por componente son informativos: no bloquean el merge.
  const componentBlock = codecovConfig.match(
      /^component_management:$\n([\s\S]*?)(?=^\S)/m);
  assert.ok(componentBlock, 'bloque component_management');
  const componentStatuses = componentBlock[1].match(
      /- type: (project|patch)\n\s+target: [^\n]+\n\s+informational: true/g);
  assert.equal(componentStatuses?.length, 2,
      'un status project y un status patch informativos');

  // Los gates de cobertura existentes no cambian.
  assert.match(codecovConfig, /^      backend:$/m);
  assert.match(codecovConfig, /^        target: 85%$/m);
  assert.match(codecovConfig, /^      frontend:$/m);
  assert.match(codecovConfig, /^        target: 80%$/m);
});
