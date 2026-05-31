/**
 * Tests for the accessibility skill files:
 * - SKILL.md: YAML frontmatter, POUR principles, conformance levels, section coverage
 * - references/A11Y-PATTERNS.md: section headings, required patterns
 * - references/WCAG.md: WCAG 2.2 criterion tables, levels, testing tools
 *
 * Uses Node.js built-in test runner (node:test). Requires Node 18+.
 * Run: node --test .agents/skills/frontend-platform/accessibility/__tests__/skill.test.mjs
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SKILL_DIR = join(__dirname, '..');
const REFS_DIR = join(SKILL_DIR, 'references');

function readFile(filePath) {
  return readFileSync(filePath, 'utf8');
}

// ─── SKILL.md ──────────────────────────────────────────────────────────────

describe('SKILL.md - file exists and is readable', () => {
  it('SKILL.md can be read', () => {
    assert.doesNotThrow(() => readFile(join(SKILL_DIR, 'SKILL.md')));
  });
});

describe('SKILL.md - YAML frontmatter', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('starts with YAML frontmatter delimiter "---"', () => {
    assert.ok(content.trimStart().startsWith('---'), 'SKILL.md should begin with YAML frontmatter "---"');
  });

  it('has a closing frontmatter delimiter', () => {
    const lines = content.split('\n');
    const closingDelimiterIndex = lines.slice(1).findIndex((l) => l.trim() === '---');
    assert.ok(closingDelimiterIndex !== -1, 'SKILL.md should have a closing "---" frontmatter delimiter');
  });

  it('frontmatter contains name field', () => {
    assert.ok(content.includes('name: accessibility'), 'SKILL.md frontmatter should include "name: accessibility"');
  });

  it('frontmatter contains a description field', () => {
    assert.match(content, /description:/, 'SKILL.md frontmatter should include a "description" field');
  });

  it('frontmatter contains license field', () => {
    assert.match(content, /license:/, 'SKILL.md frontmatter should include a "license" field');
  });

  it('frontmatter contains metadata.author field', () => {
    assert.match(content, /author:/, 'SKILL.md frontmatter should include an "author" under metadata');
  });

  it('frontmatter contains metadata.version field', () => {
    assert.match(content, /version:/, 'SKILL.md frontmatter should include a "version" under metadata');
  });

  it('license is MIT', () => {
    assert.ok(content.includes('license: MIT'), 'SKILL.md license should be MIT');
  });
});

describe('SKILL.md - WCAG POUR principles section', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('contains POUR section heading', () => {
    assert.match(content, /## WCAG Principles.*POUR/i, 'SKILL.md should have a POUR principles section');
  });

  it('mentions Perceivable principle', () => {
    assert.ok(content.includes('Perceivable'), 'SKILL.md should mention the Perceivable principle');
  });

  it('mentions Operable principle', () => {
    assert.ok(content.includes('Operable'), 'SKILL.md should mention the Operable principle');
  });

  it('mentions Understandable principle', () => {
    assert.ok(content.includes('Understandable'), 'SKILL.md should mention the Understandable principle');
  });

  it('mentions Robust principle', () => {
    assert.ok(content.includes('Robust'), 'SKILL.md should mention the Robust principle');
  });
});

describe('SKILL.md - conformance levels', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('has conformance levels section', () => {
    assert.match(content, /## Conformance levels/i, 'SKILL.md should have a Conformance levels section');
  });

  it('mentions Level A', () => {
    assert.match(content, /\*\*A\*\*/, 'SKILL.md should document Level A');
  });

  it('mentions Level AA', () => {
    assert.match(content, /\*\*AA\*\*/, 'SKILL.md should document Level AA');
  });

  it('mentions Level AAA', () => {
    assert.match(content, /\*\*AAA\*\*/, 'SKILL.md should document Level AAA');
  });
});

describe('SKILL.md - main accessibility topic sections', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('has Perceivable section', () => {
    assert.match(content, /^## Perceivable/m, 'SKILL.md should have a "## Perceivable" section');
  });

  it('has Operable section', () => {
    assert.match(content, /^## Operable/m, 'SKILL.md should have a "## Operable" section');
  });

  it('has Understandable section', () => {
    assert.match(content, /^## Understandable/m, 'SKILL.md should have a "## Understandable" section');
  });

  it('has Robust section', () => {
    assert.match(content, /^## Robust/m, 'SKILL.md should have a "## Robust" section');
  });

  it('has Testing checklist section', () => {
    assert.match(content, /## Testing checklist/i, 'SKILL.md should have a Testing checklist section');
  });
});

describe('SKILL.md - color contrast ratios documented', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('documents 4.5:1 contrast ratio for normal text (AA)', () => {
    assert.ok(content.includes('4.5:1'), 'SKILL.md should document the 4.5:1 color contrast ratio');
  });

  it('documents 3:1 contrast ratio for large text (AA)', () => {
    assert.ok(content.includes('3:1'), 'SKILL.md should document the 3:1 color contrast ratio');
  });

  it('documents 7:1 contrast ratio for enhanced (AAA)', () => {
    assert.ok(content.includes('7:1'), 'SKILL.md should document the 7:1 enhanced contrast ratio');
  });
});

describe('SKILL.md - WCAG 2.2 new criteria mentioned', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('documents Focus Not Obscured (2.4.11)', () => {
    assert.match(content, /2\.4\.11|Focus Not Obscured/i, 'SKILL.md should reference WCAG 2.4.11 Focus Not Obscured');
  });

  it('documents Target Size (2.5.8)', () => {
    assert.match(content, /2\.5\.8|Target Size/i, 'SKILL.md should reference WCAG 2.5.8 Target Size');
  });

  it('documents Dragging Movements (2.5.7)', () => {
    assert.match(content, /2\.5\.7|[Dd]ragging/i, 'SKILL.md should reference WCAG 2.5.7 Dragging Movements');
  });

  it('documents Redundant Entry (3.3.7)', () => {
    assert.match(content, /3\.3\.7|[Rr]edundant [Ee]ntry/i, 'SKILL.md should reference WCAG 3.3.7 Redundant Entry');
  });

  it('documents Accessible Authentication (3.3.8)', () => {
    assert.match(content, /3\.3\.8|[Aa]ccessible [Aa]uthentication/i, 'SKILL.md should reference WCAG 3.3.8 Accessible Authentication');
  });

  it('documents Consistent Help (3.2.6)', () => {
    assert.match(content, /3\.2\.6|[Cc]onsistent [Hh]elp/i, 'SKILL.md should reference WCAG 3.2.6 Consistent Help');
  });
});

describe('SKILL.md - visually-hidden CSS class documented', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('documents .visually-hidden CSS class', () => {
    assert.ok(content.includes('visually-hidden'), 'SKILL.md should document the visually-hidden CSS pattern');
  });

  it('visually-hidden uses position: absolute', () => {
    assert.ok(
      content.includes('position: absolute') || content.includes('position:absolute'),
      'SKILL.md visually-hidden pattern should include position: absolute'
    );
  });
});

describe('SKILL.md - prefers-reduced-motion documented', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('references prefers-reduced-motion media query', () => {
    assert.ok(
      content.includes('prefers-reduced-motion'),
      'SKILL.md should document the prefers-reduced-motion media query'
    );
  });
});

describe('SKILL.md - focus-visible documented', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('references :focus-visible CSS pseudo-class', () => {
    assert.ok(content.includes(':focus-visible'), 'SKILL.md should reference :focus-visible pseudo-class');
  });
});

describe('SKILL.md - references to companion files', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('links to A11Y-PATTERNS.md', () => {
    assert.ok(
      content.includes('A11Y-PATTERNS.md'),
      'SKILL.md should reference the A11Y-PATTERNS.md companion file'
    );
  });

  it('links to WCAG.md', () => {
    assert.ok(
      content.includes('WCAG.md'),
      'SKILL.md should reference the WCAG.md companion file'
    );
  });
});

describe('SKILL.md - target size minimum is 24x24 CSS pixels', () => {
  const content = readFile(join(SKILL_DIR, 'SKILL.md'));

  it('documents 24x24 minimum target size', () => {
    assert.ok(
      content.includes('24') && content.includes('24px'),
      'SKILL.md should document the 24x24 CSS pixel minimum target size'
    );
  });
});

// ─── references/A11Y-PATTERNS.md ───────────────────────────────────────────

describe('A11Y-PATTERNS.md - file exists and is readable', () => {
  it('A11Y-PATTERNS.md can be read', () => {
    assert.doesNotThrow(() => readFile(join(REFS_DIR, 'A11Y-PATTERNS.md')));
  });
});

describe('A11Y-PATTERNS.md - required pattern sections', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('has modal focus trap section', () => {
    assert.match(content, /## Modal focus trap/i, 'A11Y-PATTERNS.md should have a Modal focus trap section');
  });

  it('has skip link section', () => {
    assert.match(content, /## Skip link/i, 'A11Y-PATTERNS.md should have a Skip link section');
  });

  it('has error handling section', () => {
    assert.match(content, /## Error handling/i, 'A11Y-PATTERNS.md should have an Error handling section');
  });

  it('has form labels section', () => {
    assert.match(content, /## Form labels/i, 'A11Y-PATTERNS.md should have a Form labels section');
  });

  it('has dragging movements section', () => {
    assert.match(content, /## Dragging movements/i, 'A11Y-PATTERNS.md should have a Dragging movements section');
  });

  it('has ARIA tabs section', () => {
    assert.match(content, /## ARIA tabs/i, 'A11Y-PATTERNS.md should have an ARIA tabs section');
  });

  it('has live regions section', () => {
    assert.match(content, /## Live regions/i, 'A11Y-PATTERNS.md should have a Live regions section');
  });

  it('has screen reader commands section', () => {
    assert.match(content, /## Screen reader commands/i, 'A11Y-PATTERNS.md should have a Screen reader commands section');
  });
});

describe('A11Y-PATTERNS.md - modal focus trap code', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('contains Tab key handling', () => {
    assert.ok(content.includes("'Tab'") || content.includes('"Tab"'), 'Focus trap should handle the Tab key');
  });

  it('contains Escape key handling', () => {
    assert.ok(content.includes("'Escape'") || content.includes('"Escape"'), 'Focus trap should handle the Escape key');
  });

  it('contains focusable elements selector', () => {
    assert.ok(content.includes('button'), 'Focus trap should list focusable element types');
  });
});

describe('A11Y-PATTERNS.md - skip link pattern', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('skip link uses href="#main-content"', () => {
    assert.ok(
      content.includes('#main-content'),
      'Skip link pattern should reference "#main-content" anchor'
    );
  });

  it('skip link CSS moves it visually off-screen by default', () => {
    assert.ok(
      content.includes('top: -40px') || content.includes('top:-40px'),
      'Skip link should be positioned off-screen by default (top: -40px)'
    );
  });

  it('skip link CSS shows it on focus', () => {
    assert.ok(
      content.includes('.skip-link:focus') || content.includes('skip-link:focus'),
      'Skip link should be visible on :focus'
    );
  });
});

describe('A11Y-PATTERNS.md - error handling pattern', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('uses aria-invalid attribute', () => {
    assert.ok(content.includes('aria-invalid'), 'Error handling should use aria-invalid');
  });

  it('uses aria-describedby for error messages', () => {
    assert.ok(content.includes('aria-describedby'), 'Error handling should use aria-describedby');
  });

  it('uses role="alert" for error messages', () => {
    assert.ok(content.includes('role="alert"'), 'Error handling should use role="alert"');
  });

  it('focuses first error on submit', () => {
    assert.ok(
      content.includes('firstError') || content.includes('focus()'),
      'Error handling should focus the first error on form submit'
    );
  });
});

describe('A11Y-PATTERNS.md - form labels pattern', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('shows explicit label using for/id association', () => {
    assert.ok(
      content.includes('for="') && content.includes('id="'),
      'Form labels should demonstrate explicit for/id association'
    );
  });

  it('shows implicit label wrapping input', () => {
    assert.match(content, /<label>[\s\S]*?<input/m, 'Form labels should demonstrate implicit label wrapping');
  });
});

describe('A11Y-PATTERNS.md - ARIA tabs pattern', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('uses role="tablist"', () => {
    assert.ok(content.includes('role="tablist"'), 'ARIA tabs should use role="tablist"');
  });

  it('uses role="tab"', () => {
    assert.ok(content.includes('role="tab"'), 'ARIA tabs should use role="tab"');
  });

  it('uses role="tabpanel"', () => {
    assert.ok(content.includes('role="tabpanel"'), 'ARIA tabs should use role="tabpanel"');
  });

  it('uses aria-selected', () => {
    assert.ok(content.includes('aria-selected'), 'ARIA tabs should use aria-selected');
  });

  it('uses aria-controls', () => {
    assert.ok(content.includes('aria-controls'), 'ARIA tabs should use aria-controls');
  });
});

describe('A11Y-PATTERNS.md - live regions pattern', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('uses aria-live="polite" for status updates', () => {
    assert.ok(content.includes('aria-live="polite"'), 'Live regions should include aria-live="polite"');
  });

  it('uses aria-live="assertive" for urgent alerts', () => {
    assert.ok(content.includes('aria-live="assertive"'), 'Live regions should include aria-live="assertive"');
  });

  it('includes showNotification helper function', () => {
    assert.ok(
      content.includes('showNotification'),
      'Live regions should include a showNotification helper'
    );
  });
});

describe('A11Y-PATTERNS.md - screen reader commands reference table', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('includes VoiceOver commands', () => {
    assert.ok(content.includes('VoiceOver'), 'Screen reader table should include VoiceOver commands');
  });

  it('includes NVDA commands', () => {
    assert.ok(content.includes('NVDA'), 'Screen reader table should include NVDA commands');
  });
});

describe('A11Y-PATTERNS.md - dragging movements alternatives', () => {
  const content = readFile(join(REFS_DIR, 'A11Y-PATTERNS.md'));

  it('provides up/down button alternatives for drag reorder', () => {
    assert.match(
      content,
      /Move.*up|Move.*down/i,
      'Dragging movements should provide move-up and move-down button alternatives'
    );
  });
});

// ─── references/WCAG.md ────────────────────────────────────────────────────

describe('WCAG.md - file exists and is readable', () => {
  it('WCAG.md can be read', () => {
    assert.doesNotThrow(() => readFile(join(REFS_DIR, 'WCAG.md')));
  });
});

describe('WCAG.md - success criteria sections', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('has "Success criteria by level" heading', () => {
    assert.match(content, /## Success criteria by level/i, 'WCAG.md should have a Success criteria section');
  });

  it('has Level A section', () => {
    assert.match(content, /### Level A/i, 'WCAG.md should have a Level A section');
  });

  it('has Level AA section', () => {
    assert.match(content, /### Level AA/i, 'WCAG.md should have a Level AA section');
  });

  it('has Level AAA section', () => {
    assert.match(content, /### Level AAA/i, 'WCAG.md should have a Level AAA section');
  });
});

describe('WCAG.md - Level A criteria presence', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  const levelACriteria = [
    ['1.1.1', 'Non-text Content'],
    ['1.3.1', 'Info and Relationships'],
    ['1.4.1', 'Use of Color'],
    ['2.1.1', 'Keyboard'],
    ['2.1.2', 'No Keyboard Trap'],
    ['2.4.1', 'Bypass Blocks'],
    ['3.1.1', 'Language of Page'],
    ['3.3.1', 'Error Identification'],
    ['3.3.2', 'Labels or Instructions'],
    ['4.1.2', 'Name, Role, Value'],
  ];

  for (const [number, name] of levelACriteria) {
    it(`documents criterion ${number} ${name}`, () => {
      assert.ok(
        content.includes(number),
        `WCAG.md should document criterion ${number} (${name})`
      );
    });
  }
});

describe('WCAG.md - Level AA criteria presence', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  const levelAACriteria = [
    ['1.4.3', 'Contrast Minimum'],
    ['1.4.11', 'Non-text Contrast'],
    ['2.4.7', 'Focus Visible'],
    ['2.4.11', 'Focus Not Obscured'],
    ['2.5.7', 'Dragging Movements'],
    ['2.5.8', 'Target Size Minimum'],
    ['3.3.8', 'Accessible Authentication'],
    ['4.1.3', 'Status Messages'],
  ];

  for (const [number, name] of levelAACriteria) {
    it(`documents criterion ${number} ${name}`, () => {
      assert.ok(
        content.includes(number),
        `WCAG.md should document Level AA criterion ${number} (${name})`
      );
    });
  }
});

describe('WCAG.md - WCAG 2.2 changes section', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('has "What changed from 2.1 to 2.2" section', () => {
    assert.match(
      content,
      /What changed from 2\.1 to 2\.2/i,
      'WCAG.md should have a section on what changed from 2.1 to 2.2'
    );
  });

  it('documents removed criterion 4.1.1 Parsing', () => {
    assert.ok(content.includes('4.1.1'), 'WCAG.md should note the removal of 4.1.1 Parsing');
    assert.match(content, /Removed|removed/, 'WCAG.md should indicate 4.1.1 was removed');
  });

  it('documents added criteria', () => {
    assert.match(content, /Added|added/, 'WCAG.md should indicate which criteria were added in 2.2');
  });
});

describe('WCAG.md - common ARIA patterns section', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('has Common ARIA patterns section', () => {
    assert.match(content, /## Common ARIA patterns/i, 'WCAG.md should have a Common ARIA patterns section');
  });

  it('shows button pattern', () => {
    assert.match(content, /### Buttons/i, 'WCAG.md should have a Buttons subsection');
  });

  it('shows form fields pattern', () => {
    assert.match(content, /### Form fields/i, 'WCAG.md should have a Form fields subsection');
  });

  it('shows error states pattern', () => {
    assert.match(content, /### Error states/i, 'WCAG.md should have an Error states subsection');
  });

  it('shows navigation pattern', () => {
    assert.match(content, /### Navigation/i, 'WCAG.md should have a Navigation subsection');
  });

  it('shows modals pattern', () => {
    assert.match(content, /### Modals/i, 'WCAG.md should have a Modals subsection');
  });

  it('shows live regions pattern', () => {
    assert.match(content, /### Live regions/i, 'WCAG.md should have a Live regions subsection');
  });
});

describe('WCAG.md - testing tools section', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('has Testing tools section', () => {
    assert.match(content, /## Testing tools/i, 'WCAG.md should have a Testing tools section');
  });

  it('documents axe DevTools', () => {
    assert.ok(content.includes('axe'), 'WCAG.md testing tools should mention axe');
  });

  it('documents Lighthouse', () => {
    assert.ok(content.includes('Lighthouse'), 'WCAG.md testing tools should mention Lighthouse');
  });

  it('documents WAVE', () => {
    assert.ok(content.includes('WAVE'), 'WCAG.md testing tools should mention WAVE');
  });

  it('documents NVDA screen reader', () => {
    assert.ok(content.includes('NVDA'), 'WCAG.md testing tools should mention NVDA');
  });

  it('documents VoiceOver screen reader', () => {
    assert.ok(content.includes('VoiceOver'), 'WCAG.md testing tools should mention VoiceOver');
  });
});

describe('WCAG.md - sources section', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('has Sources section', () => {
    assert.match(content, /## Sources/i, 'WCAG.md should have a Sources section');
  });

  it('references WCAG 2.2 W3C Recommendation', () => {
    assert.ok(
      content.includes('WCAG 2.2') || content.includes('wcag22'),
      'WCAG.md should reference the WCAG 2.2 spec'
    );
  });
});

describe('WCAG.md - aria-current for navigation active state', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('shows aria-current="page" for active nav link', () => {
    assert.ok(
      content.includes('aria-current="page"'),
      'WCAG.md navigation pattern should demonstrate aria-current="page"'
    );
  });
});

describe('WCAG.md - external link accessibility', () => {
  const content = readFile(join(REFS_DIR, 'WCAG.md'));

  it('shows target="_blank" with rel="noopener"', () => {
    assert.ok(
      content.includes('target="_blank"') && content.includes('rel="noopener"'),
      'WCAG.md should show secure external link pattern with rel="noopener"'
    );
  });

  it('indicates external links open in new tab for screen readers', () => {
    assert.ok(
      content.includes('opens in new tab') || content.includes('new tab'),
      'WCAG.md should provide text for screen readers about links opening in new tabs'
    );
  });
});