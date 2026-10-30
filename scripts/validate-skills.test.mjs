'use strict';

import assert from 'node:assert/strict';
import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync
} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterEach, describe, it} from 'node:test';
import {analyzeSkills, validateSkills} from './validate-skills.mjs';

const roots = [];

function fixture(tree) {
  const root = mkdtempSync(join(tmpdir(), 'validate-skills-'));
  roots.push(root);
  for (const [path, content] of Object.entries(tree)) {
    const target = join(root, path);
    mkdirSync(join(target, '..'), { recursive: true });
    writeFileSync(target, content);
  }
  return root;
}

function skill(name, { description = 'Does useful work.', body = '# Skill\n' } = {}) {
  return `---\nname: ${name}\ndescription: ${description}\n---\n\n${body}`;
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('validateSkills', () => {
  it('accepts a skill whose identity matches its directory', () => {
    const root = fixture({ 'tui/ratatui/SKILL.md': skill('ratatui') });
    assert.deepEqual(validateSkills(root), []);
  });

  it('reads block scalar descriptions', () => {
    const root = fixture({
      'orchestration/SKILL.md': '---\nname: orchestration\ndescription: >\n  First line\n  second line.\n---\n',
    });
    assert.deepEqual(validateSkills(root), []);
  });

  it('rejects a name that does not match the directory', () => {
    const root = fixture({ 'astrolicious-astro/SKILL.md': skill('astro') });
    assert.match(validateSkills(root).join('\n'), /does not match directory "astrolicious-astro"/);
  });

  it('rejects missing frontmatter fields', () => {
    const root = fixture({ 'empty/SKILL.md': '---\nname: empty\n---\n' });
    assert.match(validateSkills(root).join('\n'), /missing "description"/);
  });

  it('rejects names that are not lowercase hyphenated words', () => {
    const root = fixture({ 'Bad_Name/SKILL.md': skill('Bad_Name') });
    const errors = validateSkills(root).join('\n');
    assert.match(errors, /must be lowercase alphanumeric words separated by hyphens/);
  });

  it('rejects comma-separated allowed-tools', () => {
    const root = fixture({
      'one/SKILL.md': '---\nname: one\ndescription: Use when needed.\nallowed-tools: Read, Edit, Bash\n---\n',
    });
    assert.match(validateSkills(root).join('\n'),
        /allowed-tools must be space-separated/);
  });

  it('accepts space-separated allowed-tools', () => {
    const root = fixture({
      'one/SKILL.md': '---\nname: one\ndescription: Use when needed.\nallowed-tools: Read Bash(git:*)\n---\n',
    });
    assert.deepEqual(validateSkills(root), []);
  });

  it('rejects duplicate skill names', () => {
    const root = fixture({
      'one/SKILL.md': skill('shared'),
      'two/SKILL.md': skill('shared'),
    });
    assert.match(validateSkills(root).join('\n'), /duplicate name "shared"/);
  });

  it('rejects broken relative links but accepts existing files and URLs', () => {
    const root = fixture({
      'one/SKILL.md': skill('one', {
        body: '[missing](references/missing.md) [present](references/present.md) [web](https://example.com)\n',
      }),
      'one/references/present.md': '# Present\n',
    });
    const errors = validateSkills(root).join('\n');
    assert.match(errors, /broken local link "references\/missing\.md"/);
    assert.doesNotMatch(errors, /present\.md/);
  });

  it('validates a local link destination with one balanced parenthesized group',
      () => {
        const root = fixture({
          'one/SKILL.md': skill('one',
              {body: '[setup](references/setup(v2).md)\n'}),
          'one/references/setup(v2).md': '# Setup v2\n',
        });
        assert.deepEqual(validateSkills(root), []);
      });

  it('rejects malformed UTF-8 and continues validating later skills', () => {
    const root = fixture(
        {'bad/SKILL.md': skill('bad'), 'good/SKILL.md': skill('good')});
    writeFileSync(join(root, 'bad/SKILL.md'),
        Buffer.from([0x2d, 0x2d, 0x2d, 0xff]));
    const result = analyzeSkills(root);
    assert.match(result.errors.join('\n'), /bad\/SKILL\.md:.*UTF-8/);
    assert.equal(result.skills.some(({name}) => name === 'good'), true);
  });

  it('reports symlinked skill directories instead of silently skipping them',
      () => {
        const root = fixture({'target/SKILL.md': skill('target')});
        symlinkSync(join(root, 'target'), join(root, 'linked'));
        assert.match(validateSkills(root).join('\n'),
            /linked: symlinked entries are not supported/);
      });

  it('resolves links written from the repository root', () => {
    const root = fixture({
      'skills/one/SKILL.md': skill('one', { body: '[pkg](package.json)\n' }),
      'package.json': '{}\n',
    });
    assert.deepEqual(analyzeSkills(join(root, 'skills'), root).errors, []);
  });

  it('resolves repository-root links against the injected fixture root', () => {
    const root = fixture({
      'skills/one/SKILL.md': skill('one', { body: '[pkg](package.json)\n' }),
    });
    const result = analyzeSkills(join(root, 'skills'), root);
    assert.match(result.errors.join('\n'), /broken local link "package\.json"/);
  });

  it('rejects skills larger than the documented limit', () => {
    const root = fixture({
      'big/SKILL.md': skill('big', { body: 'x'.repeat(65_536) }),
    });
    assert.match(validateSkills(root).join('\n'), /exceeds 65536 bytes/);
  });

  it('ignores markdown files that are not skills', () => {
    const root = fixture({ 'README.md': '# Index\n', 'one/SKILL.md': skill('one') });
    assert.deepEqual(validateSkills(root), []);
  });

  it('marks a direct child as discoverable and loadable without warnings', () => {
    const root = fixture({ 'one/SKILL.md': skill('one') });
    const { errors, warnings, skills } = analyzeSkills(root);
    assert.deepEqual(errors, []);
    assert.deepEqual(warnings, []);
    assert.equal(skills[0].discoverable, true);
    assert.equal(skills[0].loadable, true);
  });

  it('warns when a valid skill is nested below the flat catalog', () => {
    const root = fixture({ 'tui/ratatui/SKILL.md': skill('ratatui') });
    const { errors, warnings, skills } = analyzeSkills(root);
    assert.deepEqual(errors, []);
    assert.equal(skills[0].discoverable, false);
    assert.match(warnings.join('\n'), /not discoverable by Huginn's flat catalog/);
  });

  it('warns when a valid skill exceeds the load_skill JSON budget', () => {
    const root = fixture({
      'big/SKILL.md': skill('big', { body: 'x'.repeat(16_500) }),
    });
    const { errors, warnings, skills } = analyzeSkills(root);
    assert.deepEqual(errors, []);
    assert.equal(skills[0].loadable, false);
    assert.match(warnings.join('\n'), /valid but not loadable/);
  });

  it('keeps warnings separate from errors', () => {
    const root = fixture({ 'tui/broken/SKILL.md': skill('wrong-name') });
    const { errors, warnings } = analyzeSkills(root);
    assert.match(errors.join('\n'), /does not match directory "broken"/);
    assert.match(warnings.join('\n'), /not discoverable/);
  });
});
