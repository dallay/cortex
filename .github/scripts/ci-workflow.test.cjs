'use strict';

const {it} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {evaluateRequiredChecks} = require('./ci-required.cjs');

const workflow = fs.readFileSync(path.join(__dirname, '../workflows/ci.yml'),
    'utf8');
const categories = ['backend', 'doctor', 'frontend', 'docker', 'github', 'docs',
  'labeler', 'muninn_c1'];

function jobBlock(id) {
  const block = workflow.match(
      new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [\\w-]+:|$(?![\\s\\S]))`, 'm'));
  assert.ok(block, `job ${id}`);
  return block[1];
}

// Extrae el bloque YAML que empieza en la cabecera `header` y continúa con
// líneas más indentadas. Aislar la sección evita que una aserción dependa
// de un valor duplicado en otra parte del archivo.
function yamlBlock(text, header) {
  const lines = text.split('\n');
  const start = lines.findIndex((line) => header.test(line));
  if (start === -1) return null;
  const indent = lines[start].length - lines[start].trimStart().length;
  const block = [lines[start]];
  for (let i = start + 1; i < lines.length; i += 1) {
    const line = lines[i];
    if (line.trim() === '') {
      block.push(line);
      continue;
    }
    if (line.length - line.trimStart().length <= indent) break;
    block.push(line);
  }
  return block.join('\n');
}

// Devuelve el bloque YAML en la ruta de claves indicada, por ejemplo
// ['coverage', 'status', 'project', 'backend'].
function yamlPath(text, keys) {
  let current = text;
  for (const key of keys) {
    current = yamlBlock(current, new RegExp(`^\\s*${key}:\\s*$`));
    if (current === null) return null;
  }
  return current;
}

// Extrae los valores capturados de una lista YAML, por ejemplo `- "apps/**"`.
function yamlListItems(block, item) {
  if (block === null) return null;
  return block.split('\n')
      .map((line) => item.exec(line))
      .filter((match) => match !== null)
      .map((match) => match[1]);
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
        `unexpected token in expression: ${expression.slice(offset)}`);
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
    assert.ok(token !== undefined, 'expected literal');
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
    assert.equal(typeof left, 'boolean', 'expected a comparison');
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
  assert.equal(position, tokens.length, 'remaining tokens');
  return result;
}

function condition(id, selected, eventName = 'pull_request') {
  const block = jobBlock(id);
  let expression = block.match(/^    if: (.+)$/m)?.[1];
  if (expression === '|') {
    expression = block.match(
        /^    if: \|\n((?:      .+\n)+)/m)?.[1];
  }
  assert.ok(expression, `condition for ${id}`);
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
  ['apps/rook/new-directory/data.bin', ['backend']],
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
  ['.github/ci-path-filters.json', ['github', 'muninn_c1']],
  ['dev/muninn/product_tck.py', ['backend', 'muninn_c1']],
  ['dev/muninn/tests/test_product_tck.py', ['backend', 'muninn_c1']],
  ['dev/muninn/c1_runner.py', ['backend', 'muninn_c1']],
  ['dev/muninn/c1_attempt_plan.json', ['backend', 'muninn_c1']],
  ['dev/muninn/api_inventory.py', ['backend']],
  ['dev/muninn/tests/test_api_baseline_profiles.py', ['backend', 'muninn_c1']],
  ['crates/muninn/c1-exporter/src/lib.rs', ['backend', 'muninn_c1']],
  ['crates/muninn/application/tests/short_term_flows.rs', ['backend', 'muninn_c1']],
  ['apps/muninn/src/main.rs', ['backend', 'muninn_c1']],
  ['third_party/candle-core/src/lib.rs', ['backend', 'muninn_c1']],
  ['dev/muninn/fixtures/artifact-lock.json', ['muninn_c1']],
  ['docs/muninn/specs/tck-baseline.csv', ['docs', 'muninn_c1']],
  ['docs/muninn/reference-tck.md', ['docs']],
  ['.agents/rpi/goal/muninn-functional-parity/attempt68-bundle-verify.jsonl', ['docs', 'muninn_c1']],
  ['.github/workflows/ci.yml', ['github', 'muninn_c1']],
  ['apps/rook/src/main.rs', ['backend']],
  ['.github/ci-quality-policy.json', ['github']],
  ['.config/nextest.toml', ['backend']],
  ['.cargo/config.toml', ['backend']],
  // The whole package includes the dashboard: extra backend checks are accepted.
  ['apps/rook/dashboard/src/main.tsx', ['frontend', 'backend']],
  ['apps/huginn/src/main.rs', ['backend']],
  ['crates/rook-core/src/lib.rs', ['backend']],
  ['Dockerfile.dev', ['docker']],
  ['apps/rook/Dockerfile', ['backend', 'docker']],
  ['dev/Dockerfile.runtime', ['docker']],
  ['new/directory/Dockerfile.ci', ['docker']],
  ['dev/e2e/run-api-keys-e2e.sh', ['docker']],
  ['apps/rook/dashboard/e2e/login.spec.ts', ['frontend', 'docker']],
  ['docs/development.md', ['docs']],
];

it('covers the whole Rook package without listing directories', () => {
  const filters = JSON.parse(
      fs.readFileSync(path.join(__dirname, '../ci-path-filters.json'), 'utf8'));
  // Dorny uses picomatch with dot:true; this pattern also covers hidden paths.
  // matchesGlob is not used for hidden paths because Node applies different semantics.
  assert.ok(filters.backend.includes('apps/rook/**'));
});

for (const [file, expected] of routeCases) {
  it(`routes ${file} to its checks`, () => {
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
            'clippy', 'check', 'test', 'test-muninn-c1', 'test-frontend', 'coverage',
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

it('uses the filters tested by the detector', () => {
  assert.match(jobBlock('changes'),
      /filters:\s*\.github\/ci-path-filters\.json\s*$/m);
  for (const category of categories) {
    assert.ok(jobBlock('changes').includes(
        `${category}: \${{ steps.filter.outputs.${category} }}`));
    assert.ok(jobBlock('required').includes(
        `CHANGES_${category.toUpperCase()}: \${{ needs.changes.outputs.${category} }}`));
  }
});

it('runs Sonar and its dashboard input for documentation or metadata PRs',
    () => {
      for (const selected of
          [Object.fromEntries(categories.map((key) => [key, 'false'])),
            Object.fromEntries(categories.map(
                (key) => [key, key === 'docs' ? 'true' : 'false']))]) {
        for (const id of ['sonar', 'build-dashboard']) {
          assert.equal(condition(id, selected), true, `${id} on PR`);
          assert.equal(condition(id, selected, 'push'), false,
              `${id} on push without code`);
        }
      }
    });

it('validates SONAR_TOKEN before running PR actions and has no exceptions',
    () => {
      const sonar = jobBlock('sonar');
      const validation = sonar.match(
          /^        run: \|\n((?:          .*\n)+)/m)?.[1];
      assert.ok(validation);
      assert.ok(sonar.indexOf('run: |') < sonar.indexOf('uses:'),
          'validation before PR actions and code');
      assert.match(validation, /if \[ -z "\$SONAR_TOKEN" \]; then/);
      assert.match(validation, /exit 1/);
      assert.doesNotMatch(sonar, /sonar_exempt|continue-on-error:/);
    });

it('waits for the quality gate for 300 seconds and keeps both coverage reports',
    () => {
      const sonar = jobBlock('sonar');
      assert.match(sonar, /^    timeout-minutes: 60\s*$/m);
      assert.match(sonar, /-Dsonar\.qualitygate\.wait=true/);
      assert.match(sonar, /-Dsonar\.qualitygate\.timeout=300/);
      assert.match(sonar, /pnpm exec vitest run --coverage/);
      assert.match(sonar,
          /cargo llvm-cov report --lcov --output-path coverage\/rust-lcov\.info/);
      const scan = sonar.slice(sonar.indexOf('      - name: SonarQube Scan'));
      assert.doesNotMatch(scan, /^        if:/m);
      assert.doesNotMatch(workflow, /pull_request_target/);
      assert.match(sonar, /permissions:\n      contents: read\n    steps:/);
    });

it('keeps the 87% Rust threshold and includes the Muninn product packages', () => {
  const coverage = jobBlock('coverage');
  assert.match(coverage, /cargo llvm-cov clean --workspace/);
  assert.match(coverage, /cargo llvm-cov --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn --no-report/);
  assert.match(coverage, /cargo llvm-cov -p muninn-providers -p muninn --no-report/);
  assert.match(coverage, /cargo llvm-cov report --fail-under-lines 87 --lcov --output-path lcov\.info/);
  const sonar = jobBlock('sonar');
  assert.match(sonar, /cargo llvm-cov clean --workspace/);
  assert.match(sonar, /cargo llvm-cov --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn --no-report/);
  assert.match(sonar, /cargo llvm-cov -p muninn-providers -p muninn --no-report/);
  assert.match(sonar, /cargo llvm-cov report --lcov --output-path coverage\/rust-lcov\.info/);
});

it('Gitleaks has the full history without persisting credentials', () => {
  const gitleaks = jobBlock('gitleaks-pr');
  assert.match(gitleaks, /fetch-depth: 0\s*\n\s*persist-credentials: false/);
  assert.match(gitleaks,
      /BASE_SHA: \$\{\{ github\.event\.pull_request\.base\.sha \}\}/);
  assert.match(gitleaks,
      /HEAD_SHA: \$\{\{ github\.event\.pull_request\.head\.sha \|\| github\.sha \}\}/);
  assert.doesNotMatch(gitleaks, /--all -n 50|fetch-depth: 50/);
});

for (const eventName of ['pull_request', 'push']) {
  it(`Gitleaks configures the full range on ${eventName} without remote operations`,
      () => {
        const scan = jobBlock('gitleaks-pr').match(
            /^      - name: Analyze commit history\n([\s\S]*?)(?=^      - )/m)?.[1];
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

it('fails only on active Semgrep findings, not on SARIF exceptions in source',
    () => {
      const scan = jobBlock('semgrep-pr');
      assert.match(scan, /FINDINGS_COUNT=.*suppressions/);
      assert.match(scan,
          /select\(\(\.suppressions \/\/ \[\]\) \| length == 0\)/);
    });

it('pins the Semgrep toolchain instead of inheriting the runner Python', () => {
  const scan = jobBlock('semgrep-pr');
  assert.match(scan,
      /uses: actions\/setup-python@[0-9a-f]{40} # v5\.\d+\.\d+/);
  assert.match(scan, /python-version: '3\.11'/);
  assert.match(scan,
      /run: python -m pip install semgrep==1\.126\.0 'setuptools<81'/);
  assert.doesNotMatch(scan, /^        run: pip install semgrep/m);

  const deep = fs.readFileSync(
      path.join(__dirname, '../workflows/security-deep.yml'), 'utf8');
  const full = deep.match(
      /^  semgrep-full:\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/m)?.[1];
  assert.ok(full, 'job semgrep-full');
  assert.match(full,
      /uses: actions\/setup-python@[0-9a-f]{40} # v5\.\d+\.\d+/);
  assert.match(full, /python-version: '3\.11'/);
  assert.match(full,
      /run: python -m pip install semgrep==1\.126\.0 'setuptools<81'/);
  assert.doesNotMatch(full, /^        run: pip install semgrep/m);
});

it('authenticates the checks and the Semgrep scan with the CI secret', () => {
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

it('always runs the CI tests and requires their real result', () => {
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

it('combines default providers and app coverage in the same 87% Rust gate', () => {
  const root = path.resolve(__dirname, '../..');
  const local = fs.readFileSync(path.join(root, 'justfile'), 'utf8');
  const recipe = local.split('ci-coverage-rust:\n')[1].split('\n# ')[0];
  for (const block of [jobBlock('coverage'), jobBlock('sonar'), recipe]) {
    assert.match(block, /cargo llvm-cov clean --workspace/);
    assert.match(block, /cargo llvm-cov --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn --no-report/);
    assert.match(block, /cargo llvm-cov -p muninn-providers -p muninn --no-report/);
    assert.match(block, /cargo llvm-cov report[^\n]*--lcov/);
    assert.doesNotMatch(block, /cargo llvm-cov report[^\n]*--exclude/);
  }
  assert.match(jobBlock('coverage'), /cargo llvm-cov report --fail-under-lines 87 --lcov/);
  assert.match(recipe, /cargo llvm-cov report --fail-under-lines 87 --lcov/);
});

it('runs Muninn with default features in addition to the opt-in C1 gate', () => {
  const command = /cargo test --locked -p muninn-providers -p muninn/;
  assert.match(jobBlock('test'), command);
  assert.match(jobBlock('test-multi'), command);
  const justfile = fs.readFileSync(path.resolve(__dirname, '../../justfile'), 'utf8');
  assert.match(justfile, /cargo test --locked -p muninn-providers -p muninn/);
  assert.match(justfile, /run_step "Muninn C1 opt-in tests" just ci-muninn-c1/);
});

it('filters the C1 job by its own materials, uses cache and cancels stale runs', () => {
  const c1 = jobBlock('test-muninn-c1');
  assert.match(c1, /^    runs-on: macos-15$/m);
  assert.match(c1, /needs: \[changes\]/);
  assert.match(c1, /needs\.changes\.outputs\.muninn_c1 == 'true'/);
  assert.match(c1, /timeout-minutes: 45/);
  assert.match(c1, /concurrency:/);
  assert.match(c1, /cancel-in-progress: true/);
  assert.match(c1, /Swatinem\/rust-cache@/);
  assert.match(c1, /cargo test --locked -p muninn-c1-exporter --features cpu-f32 --target aarch64-apple-darwin/);
  assert.match(c1, /cargo test --locked -p muninn-providers -p muninn --features minilm-cpu --target aarch64-apple-darwin/);
  assert.match(c1, /permissions:\n      contents: read/);
  assert.match(jobBlock('changes'), /muninn_c1: \$\{\{ steps\.filter\.outputs\.muninn_c1 \}\}/);
});

it('attests only on protected main after the C1 test and limits permissions to the privileged job', () => {
  const attest = jobBlock('attest-muninn-c1');
  assert.match(attest, /needs: \[changes, test-muninn-c1\]/);
  assert.match(attest, /github\.event_name == 'push'/);
  assert.match(attest, /github\.ref == 'refs\/heads\/main'/);
  assert.match(attest, /github\.ref_protected == true/);
  assert.match(attest, /needs\.changes\.outputs\.muninn_c1 == 'true'/);
  assert.match(attest, /permissions:\n      contents: read\n      id-token: write\n      attestations: write/);
  const c1Test = jobBlock('test-muninn-c1');
  assert.match(c1Test, /permissions:\n      contents: read/);
  assert.doesNotMatch(c1Test, /id-token: write|attestations: write/);
  assert.doesNotMatch(jobBlock('required'), /^      - attest-muninn-c1\s*$/m);
});

it('requires Huginn Seatbelt enforcement on macOS PRs without filtering tests',
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

it('scopes the Gitleaks exception for the GM-05 fixture to its file and value',
    () => {
      const {globalSection, rules} = gitleaksRules(gitleaksConfig);
      const generic = rules.find((rule) => rule.id === 'generic_secret');
      assert.ok(generic, 'generic_secret rule present');

      const scoped = generic.allowlists.filter(
          (section) => section.includes('dev/muninn'));
      assert.equal(scoped.length, 1,
          'only one rule allowlist mentions dev/muninn');
      const block = scoped[0];
      assert.ok(block.includes('condition = "AND"'),
          'AND condition to scope the match');
      assert.ok(block.includes('regexTarget = "match"'),
          'the target is the full match');
      assert.ok(
          block.includes(
              `paths = ['''^dev/muninn/tests/test_gm05_build\\.py$''']`),
          'exact path of the GM-05 test',
      );
      assert.ok(
          block.includes(
              `regexes = ['''(?i)^token\\s*=\\s*"synthetic-test-value$''']`),
          'exact value of the synthetic token',
      );
      assert.doesNotMatch(block, /condition = "OR"/);

      for (const rule of
          rules.filter((candidate) => candidate.id !== 'generic_secret')) {
        assert.ok(
            !rule.allowlists.some(
                (section) => section.includes('dev/muninn') || section.includes(
                    'synthetic-test-value')),
            `rule ${rule.id} declares no exceptions for the GM-05 fixture`,
        );
      }

      assert.doesNotMatch(globalSection, /dev\/muninn|synthetic-test-value/,
          'no global allowlist for the fixture');
    });

it('scopes the Seatbelt synthetic markers to their file and values', () => {
  const {globalSection, rules} = gitleaksRules(gitleaksConfig);
  const generic = rules.find((rule) => rule.id === 'generic_secret');
  assert.ok(generic, 'generic_secret rule present');

  const scoped = generic.allowlists.filter(
      (section) => section.includes('crates/huginn/runtime/src/tools'));
  assert.equal(scoped.length, 1, 'one scoped allowlist for tools.rs');
  const block = scoped[0];
  assert.ok(block.includes('condition = "AND"'),
      'AND condition to scope the match');
  assert.ok(block.includes('regexTarget = "match"'),
      'the target is the full match');
  assert.ok(
      block.includes(
          `paths = ['''^crates/huginn/runtime/src/tools\\.rs$''']`),
      'exact path of the tools module',
  );
  assert.ok(
      block.includes(
          `'''(?i)^secret\\s*=\\s*"(other-workspace-secret-value|credential-fixture-secret-not-real|external-fixture-secret|unassigned-temp-fixture-secret|cwd-does-not-expand-permissions|symlink-target-fixture-secret)$'''`),
      'only the synthetic markers of the denial tests',
  );
  assert.doesNotMatch(block, /condition = "OR"/);

  for (const rule of
      rules.filter((candidate) => candidate.id !== 'generic_secret')) {
    assert.ok(
        !rule.allowlists.some(
            (section) => section.includes('crates/huginn/runtime/src/tools')),
        `rule ${rule.id} declares no Seatbelt exceptions`,
    );
  }

  assert.doesNotMatch(globalSection, /crates\/huginn\/runtime\/src\/tools\.rs/,
      'no global allowlist for the Seatbelt fixtures');
});

it('connects every Required dependency to the runner without losing results',
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

it('generates and uploads Rust JUnit results without losing doctests', () => {
  const nextestConfigPath = path.join(__dirname, '../../.config/nextest.toml');
  assert.ok(fs.existsSync(nextestConfigPath), 'Nextest CI profile present');
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

it('generates and uploads Vitest JUnit results after the frontend job', () => {
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

it('enables Bundle Analysis only for frontend changes and scopes the token', () => {
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

it('publishes coverage comments early and keeps the carryforward flags', () => {
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

  const components = yamlPath(codecovConfig,
      ['component_management', 'individual_components']);
  assert.ok(components, 'bloque individual_components');

  // Cada componente declara las rutas de su producto. Las rutas se validan
  // dentro del bloque del componente: los flags pueden compartir rutas y un
  // match global no detectaría una atribución equivocada.
  const expectedPaths = {
    rook: [
      'apps/rook/src/**',
      'apps/rook/tests/**',
      'crates/domain/**',
      'crates/application/**',
      'crates/infrastructure/**',
      'crates/test-support/**',
    ],
    huginn: [
      'apps/huginn/src/**',
      'apps/huginn/tests/**',
      'crates/huginn/**',
    ],
    dashboard: [
      'apps/rook/dashboard/**',
    ],
  };

  for (const [componentId, paths] of Object.entries(expectedPaths)) {
    const component = yamlBlock(components,
        new RegExp(`^\\s*- component_id: ${componentId}$`));
    assert.ok(component, `componente ${componentId}`);
    const declared = yamlListItems(component, /^\s*-\s+"(.*)"$/m);
    assert.deepEqual(declared, paths, `rutas del componente ${componentId}`);
  }

  // Los estados por componente son informativos: no bloquean el merge.
  const statuses = yamlPath(codecovConfig,
      ['component_management', 'default_rules', 'statuses']);
  assert.ok(statuses, 'bloque statuses de component_management');
  for (const type of ['project', 'patch']) {
    const status = yamlBlock(statuses, new RegExp(`^\\s*- type: ${type}$`));
    assert.ok(status, `status ${type} del componente`);
    assert.match(status, /^\s*informational: true$/m,
        `el status ${type} del componente es informativo`);
  }

  // Los gates de cobertura por producto no cambian. Cada target se valida
  // dentro de su bloque: project y patch declaran umbrales distintos.
  const expectedTargets = {
    'coverage/status/project/backend': '85%',
    'coverage/status/project/frontend': '80%',
    'coverage/status/patch/backend': '85%',
    'coverage/status/patch/frontend': '75%',
  };

  for (const [blockPath, target] of Object.entries(expectedTargets)) {
    const block = yamlPath(codecovConfig, blockPath.split('/'));
    assert.ok(block, `bloque ${blockPath}`);
    assert.match(block, new RegExp(`^\\s*target: ${target}$`, 'm'),
        `target de ${blockPath}`);
  }
});
