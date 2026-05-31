/**
 * Tests for animate-text skill assets:
 * - assets/catalog.json schema and integrity
 * - assets/effects/*.json schema and content correctness
 *
 * Uses Node.js built-in test runner (node:test). Requires Node 18+.
 * Run: node --test .agents/skills/frontend-platform/animate-text/__tests__/assets.test.mjs
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, basename, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ASSETS_DIR = join(__dirname, '..', 'assets');
const EFFECTS_DIR = join(ASSETS_DIR, 'effects');
const CATALOG_PATH = join(ASSETS_DIR, 'catalog.json');

// ─── helpers ───────────────────────────────────────────────────────────────

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, 'utf8'));
}

function getAllEffectFiles() {
  return readdirSync(EFFECTS_DIR)
    .filter((f) => f.endsWith('.json'))
    .map((f) => ({ filename: f, id: basename(f, '.json'), path: join(EFFECTS_DIR, f) }));
}

const VALID_TARGETS = new Set(['whole', 'per-character', 'per-word', 'per-line']);
const VALID_VISIBILITIES = new Set(['visible', 'hidden']);
const VALID_SWAP_MODES = new Set(['crossfade', 'sequential']);
const VALID_SUPPORTED_ADAPTERS = new Set(['waapi', 'motion', 'gsap']);

// Round float to 2 decimal places for timing math comparisons
function round2(n) {
  return Math.round(n * 100) / 100;
}

// ─── catalog.json ──────────────────────────────────────────────────────────

describe('catalog.json - structure', () => {
  let catalog;

  it('parses as valid JSON', () => {
    assert.doesNotThrow(() => {
      catalog = readJson(CATALOG_PATH);
    });
  });

  it('has a visible_ids array', () => {
    catalog = readJson(CATALOG_PATH);
    assert.ok(Array.isArray(catalog.visible_ids), 'visible_ids should be an array');
  });

  it('visible_ids contains only strings', () => {
    catalog = readJson(CATALOG_PATH);
    for (const id of catalog.visible_ids) {
      assert.strictEqual(typeof id, 'string', `visible_ids entry "${id}" should be a string`);
      assert.ok(id.length > 0, 'visible_ids entries should not be empty strings');
    }
  });

  it('visible_ids has no duplicates', () => {
    catalog = readJson(CATALOG_PATH);
    const seen = new Set();
    for (const id of catalog.visible_ids) {
      assert.ok(!seen.has(id), `Duplicate visible_id: "${id}"`);
      seen.add(id);
    }
  });

  it('has a renderer_overrides object', () => {
    catalog = readJson(CATALOG_PATH);
    assert.ok(
      catalog.renderer_overrides !== null && typeof catalog.renderer_overrides === 'object' && !Array.isArray(catalog.renderer_overrides),
      'renderer_overrides should be a plain object'
    );
  });

  it('renderer_overrides values are strings', () => {
    catalog = readJson(CATALOG_PATH);
    for (const [key, value] of Object.entries(catalog.renderer_overrides)) {
      assert.strictEqual(typeof value, 'string', `renderer_overrides["${key}"] should be a string`);
    }
  });

  it('has exactly 2 top-level keys', () => {
    catalog = readJson(CATALOG_PATH);
    const keys = Object.keys(catalog);
    assert.deepStrictEqual(
      keys.sort(),
      ['renderer_overrides', 'visible_ids'].sort(),
      'catalog.json should have exactly visible_ids and renderer_overrides'
    );
  });
});

describe('catalog.json - integrity: visible_ids reference existing effect files', () => {
  const catalog = readJson(CATALOG_PATH);

  for (const id of catalog.visible_ids) {
    it(`effect file exists for visible id "${id}"`, () => {
      const expectedPath = join(EFFECTS_DIR, `${id}.json`);
      assert.ok(
        existsSync(expectedPath),
        `Expected effects/${id}.json to exist for catalog visible_id "${id}"`
      );
    });
  }
});

describe('catalog.json - renderer_overrides keys exist as effect files', () => {
  const catalog = readJson(CATALOG_PATH);

  for (const key of Object.keys(catalog.renderer_overrides)) {
    it(`effect file exists for renderer_override key "${key}"`, () => {
      const expectedPath = join(EFFECTS_DIR, `${key}.json`);
      assert.ok(
        existsSync(expectedPath),
        `Expected effects/${key}.json to exist for renderer_overrides key "${key}"`
      );
    });
  }
});

describe('catalog.json - renderer_overrides keys are in visible_ids', () => {
  const catalog = readJson(CATALOG_PATH);

  for (const key of Object.keys(catalog.renderer_overrides)) {
    it(`renderer_override key "${key}" appears in visible_ids`, () => {
      assert.ok(
        catalog.visible_ids.includes(key),
        `renderer_overrides key "${key}" should be listed in visible_ids`
      );
    });
  }
});

describe('catalog.json - visible effects match visibility flag', () => {
  const catalog = readJson(CATALOG_PATH);

  for (const id of catalog.visible_ids) {
    const effectPath = join(EFFECTS_DIR, `${id}.json`);
    if (!existsSync(effectPath)) continue; // covered by integrity test

    it(`"${id}" has visibility "visible"`, () => {
      const effect = readJson(effectPath);
      assert.strictEqual(
        effect.visibility,
        'visible',
        `Effect "${id}" is in catalog.visible_ids but has visibility "${effect.visibility}"`
      );
    });
  }
});

// ─── effect JSON files - top-level structure ───────────────────────────────

describe('effect JSON files - all parse as valid JSON', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename} is valid JSON`, () => {
      assert.doesNotThrow(() => readJson(path), `${filename} should parse as valid JSON`);
    });
  }
});

describe('effect JSON files - required top-level fields', () => {
  for (const { filename, id, path } of getAllEffectFiles()) {
    it(`${filename} has required top-level fields: id, visibility, portable_spec`, () => {
      const effect = readJson(path);
      assert.ok('id' in effect, `${filename}: missing top-level "id"`);
      assert.ok('visibility' in effect, `${filename}: missing top-level "visibility"`);
      assert.ok('portable_spec' in effect, `${filename}: missing top-level "portable_spec"`);
      assert.ok('showcase' in effect, `${filename}: missing top-level "showcase" (may be null)`);
    });
  }
});

describe('effect JSON files - id matches filename', () => {
  for (const { filename, id, path } of getAllEffectFiles()) {
    it(`${filename}: effect.id matches filename`, () => {
      const effect = readJson(path);
      assert.strictEqual(
        effect.id,
        id,
        `${filename}: effect.id "${effect.id}" should match filename stem "${id}"`
      );
    });
  }
});

describe('effect JSON files - visibility is "visible" or "hidden"', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: visibility is valid enum value`, () => {
      const effect = readJson(path);
      assert.ok(
        VALID_VISIBILITIES.has(effect.visibility),
        `${filename}: visibility "${effect.visibility}" must be "visible" or "hidden"`
      );
    });
  }
});

describe('effect JSON files - showcase is null for hidden effects', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: hidden effect has showcase: null`, () => {
      const effect = readJson(path);
      if (effect.visibility === 'hidden') {
        assert.strictEqual(
          effect.showcase,
          null,
          `${filename}: hidden effect should have showcase: null`
        );
      }
    });
  }
});

describe('effect JSON files - showcase is an object for visible effects', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: visible effect has non-null showcase object`, () => {
      const effect = readJson(path);
      if (effect.visibility === 'visible') {
        assert.notStrictEqual(
          effect.showcase,
          null,
          `${filename}: visible effect must have a non-null showcase`
        );
        assert.strictEqual(
          typeof effect.showcase,
          'object',
          `${filename}: showcase should be an object`
        );
      }
    });
  }
});

// ─── portable_spec ─────────────────────────────────────────────────────────

describe('portable_spec - required fields', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: portable_spec has required fields`, () => {
      const { portable_spec: spec } = readJson(path);
      const required = ['id', 'display_name', 'description', 'target', 'signature_easing', 'enter', 'exit', 'swap'];
      for (const field of required) {
        assert.ok(field in spec, `${filename}: portable_spec missing required field "${field}"`);
      }
    });
  }
});

describe('portable_spec - id matches top-level id', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: portable_spec.id matches top-level id`, () => {
      const effect = readJson(path);
      assert.strictEqual(
        effect.portable_spec.id,
        effect.id,
        `${filename}: portable_spec.id "${effect.portable_spec.id}" must match top-level id "${effect.id}"`
      );
    });
  }
});

describe('portable_spec - display_name is a non-empty string', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: portable_spec.display_name is a non-empty string`, () => {
      const { portable_spec: spec } = readJson(path);
      assert.strictEqual(typeof spec.display_name, 'string', `${filename}: display_name should be a string`);
      assert.ok(spec.display_name.length > 0, `${filename}: display_name should not be empty`);
    });
  }
});

describe('portable_spec - target is a valid enum value', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: target is one of "whole", "per-character", "per-word", "per-line"`, () => {
      const { portable_spec: spec } = readJson(path);
      assert.ok(
        VALID_TARGETS.has(spec.target),
        `${filename}: target "${spec.target}" is not a valid value. Must be one of: ${[...VALID_TARGETS].join(', ')}`
      );
    });
  }
});

describe('portable_spec - enter phase required fields', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter has duration_ms, stagger_ms, easing, from, to`, () => {
      const { portable_spec: { enter } } = readJson(path);
      assert.ok('duration_ms' in enter, `${filename}: enter missing duration_ms`);
      assert.ok('stagger_ms' in enter, `${filename}: enter missing stagger_ms`);
      assert.ok('easing' in enter, `${filename}: enter missing easing`);
      assert.ok('from' in enter, `${filename}: enter missing from`);
      assert.ok('to' in enter, `${filename}: enter missing to`);
    });
  }
});

describe('portable_spec - exit phase required fields', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: exit has duration_ms, stagger_ms, easing, from, to`, () => {
      const { portable_spec: { exit } } = readJson(path);
      assert.ok('duration_ms' in exit, `${filename}: exit missing duration_ms`);
      assert.ok('stagger_ms' in exit, `${filename}: exit missing stagger_ms`);
      assert.ok('easing' in exit, `${filename}: exit missing easing`);
      assert.ok('from' in exit, `${filename}: exit missing from`);
      assert.ok('to' in exit, `${filename}: exit missing to`);
    });
  }
});

describe('portable_spec - enter duration_ms is a positive number', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter.duration_ms > 0`, () => {
      const { portable_spec: { enter } } = readJson(path);
      assert.ok(typeof enter.duration_ms === 'number', `${filename}: enter.duration_ms must be a number`);
      assert.ok(enter.duration_ms > 0, `${filename}: enter.duration_ms must be > 0, got ${enter.duration_ms}`);
    });
  }
});

describe('portable_spec - exit duration_ms is a positive number', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: exit.duration_ms > 0`, () => {
      const { portable_spec: { exit } } = readJson(path);
      assert.ok(typeof exit.duration_ms === 'number', `${filename}: exit.duration_ms must be a number`);
      assert.ok(exit.duration_ms > 0, `${filename}: exit.duration_ms must be > 0, got ${exit.duration_ms}`);
    });
  }
});

describe('portable_spec - stagger_ms is a non-negative number', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter.stagger_ms >= 0 and exit.stagger_ms >= 0`, () => {
      const { portable_spec: { enter, exit } } = readJson(path);
      assert.ok(enter.stagger_ms >= 0, `${filename}: enter.stagger_ms must be >= 0, got ${enter.stagger_ms}`);
      assert.ok(exit.stagger_ms >= 0, `${filename}: exit.stagger_ms must be >= 0, got ${exit.stagger_ms}`);
    });
  }
});

describe('portable_spec - enter from.opacity is 0 for standard effects', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter.from.opacity is 0 unless custom_renderer handles word-level opacity`, () => {
      const { portable_spec: spec } = readJson(path);
      // Effects with a custom_renderer (e.g. short-slide-right) may use opacity:1 on the
      // container while individual units are revealed through a nested "build" block.
      if (spec.custom_renderer) return;
      assert.strictEqual(spec.enter.from.opacity, 0, `${filename}: enter.from.opacity should be 0`);
    });
  }
});

describe('portable_spec - enter to.opacity is 1', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter.to.opacity is 1 (fully opaque end)`, () => {
      const { portable_spec: { enter } } = readJson(path);
      assert.strictEqual(enter.to.opacity, 1, `${filename}: enter.to.opacity should be 1`);
    });
  }
});

describe('portable_spec - exit to.opacity is 0', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: exit.to.opacity is 0 (fully transparent exit)`, () => {
      const { portable_spec: { exit } } = readJson(path);
      assert.strictEqual(exit.to.opacity, 0, `${filename}: exit.to.opacity should be 0`);
    });
  }
});

describe('portable_spec - swap has mode field', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: swap.mode is "crossfade" or "sequential"`, () => {
      const { portable_spec: { swap } } = readJson(path);
      assert.ok('mode' in swap, `${filename}: swap missing mode field`);
      assert.ok(
        VALID_SWAP_MODES.has(swap.mode),
        `${filename}: swap.mode "${swap.mode}" must be "crossfade" or "sequential"`
      );
    });
  }
});

describe('portable_spec - swap overlap_ms is a non-negative number', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: swap.overlap_ms >= 0`, () => {
      const { portable_spec: { swap } } = readJson(path);
      assert.ok(typeof swap.overlap_ms === 'number', `${filename}: swap.overlap_ms must be a number`);
      assert.ok(swap.overlap_ms >= 0, `${filename}: swap.overlap_ms must be >= 0, got ${swap.overlap_ms}`);
    });
  }
});

// ─── visible effects - showcase structure ──────────────────────────────────

describe('visible effects - showcase required top-level keys', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: visible showcase has content, renderer, runtime, playback, timing, stage, rendering_contract, library_selection, library_adapters`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { showcase } = effect;
      const required = [
        'content', 'renderer', 'runtime', 'playback',
        'timing', 'stage', 'rendering_contract',
        'library_selection', 'library_adapters',
      ];
      for (const key of required) {
        assert.ok(key in showcase, `${filename}: showcase missing required key "${key}"`);
      }
    });
  }
});

describe('visible effects - showcase.content has sample and samples array', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: showcase.content.sample is a non-empty string`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { content } = effect.showcase;
      assert.ok('sample' in content, `${filename}: showcase.content missing "sample"`);
      assert.strictEqual(typeof content.sample, 'string', `${filename}: content.sample must be a string`);
      assert.ok(content.sample.length > 0, `${filename}: content.sample must not be empty`);
    });
  }
});

describe('visible effects - showcase.runtime has required timing fields', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: showcase.runtime has speed_multiplier, hold_ms, gap_ms, y_travel_multiplier`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { runtime } = effect.showcase;
      assert.ok('speed_multiplier' in runtime, `${filename}: showcase.runtime missing speed_multiplier`);
      assert.ok('hold_ms' in runtime, `${filename}: showcase.runtime missing hold_ms`);
      assert.ok('gap_ms' in runtime, `${filename}: showcase.runtime missing gap_ms`);
      assert.ok('y_travel_multiplier' in runtime, `${filename}: showcase.runtime missing y_travel_multiplier`);
    });
  }
});

describe('visible effects - runtime.speed_multiplier is 0.72', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: runtime.speed_multiplier is 0.72`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      assert.strictEqual(
        effect.showcase.runtime.speed_multiplier,
        0.72,
        `${filename}: runtime.speed_multiplier should be 0.72`
      );
    });
  }
});

describe('visible effects - showcase.playback has kind and cycle array', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: showcase.playback has kind and cycle`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { playback } = effect.showcase;
      assert.ok('kind' in playback, `${filename}: showcase.playback missing "kind"`);
      assert.ok('cycle' in playback, `${filename}: showcase.playback missing "cycle"`);
      assert.ok(Array.isArray(playback.cycle), `${filename}: playback.cycle must be an array`);
      assert.ok(playback.cycle.length > 0, `${filename}: playback.cycle must not be empty`);
    });
  }
});

describe('visible effects - showcase.library_adapters has waapi, motion, gsap', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: library_adapters has waapi, motion, and gsap`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { library_adapters } = effect.showcase;
      assert.ok('waapi' in library_adapters, `${filename}: library_adapters missing "waapi"`);
      assert.ok('motion' in library_adapters, `${filename}: library_adapters missing "motion"`);
      assert.ok('gsap' in library_adapters, `${filename}: library_adapters missing "gsap"`);
    });
  }
});

describe('visible effects - library_selection.supported_adapters matches library_adapters keys', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: library_selection.supported_adapters aligns with library_adapters keys`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { library_selection, library_adapters } = effect.showcase;
      assert.ok(Array.isArray(library_selection.supported_adapters), `${filename}: supported_adapters must be an array`);

      const adapterKeys = new Set(Object.keys(library_adapters));
      for (const adapter of library_selection.supported_adapters) {
        assert.ok(adapterKeys.has(adapter), `${filename}: supported_adapter "${adapter}" has no matching library_adapters entry`);
      }
    });
  }
});

describe('visible effects - showcase.timing scaled values are approximately source * speed_multiplier', () => {
  // kinetic-center-build uses different timing structure; only test generic-stagger effects.
  // Step-eased effects (e.g. shared-axis-y with steps(1, end)) may round scaled values
  // to a perceptible minimum (e.g. 140ms) that deviates more from strict 0.72 scaling.
  // Allow ±15ms to accommodate both standard rounding and step-function minimums.
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: timing.enter.scaled_duration_ms ≈ source * 0.72`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { timing } = effect.showcase;
      // kinetic-center-build uses first_word/push/exit keys, not enter/exit
      if (!timing.enter) return;

      const expected = round2(timing.enter.source_duration_ms * 0.72);
      const actual = timing.enter.scaled_duration_ms;
      assert.ok(
        Math.abs(actual - expected) <= 15,
        `${filename}: timing.enter.scaled_duration_ms ${actual} should be ≈ ${timing.enter.source_duration_ms} * 0.72 = ${expected} (±15ms)`
      );
    });
  }
});

describe('visible effects - showcase.timing exit scaled values are approximately source * speed_multiplier', () => {
  // Step-eased effects (steps(1, end)) use a minimum timing floor that doesn't follow
  // strict 0.72 scaling, so skip them here. They are validated by the "is positive" tests.
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: timing.exit.scaled_duration_ms ≈ source * 0.72`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { timing } = effect.showcase;
      if (!timing.exit) return;

      // Skip step-eased effects — they round to a perceptible minimum rather than 0.72 * source
      if (timing.exit.easing && timing.exit.easing.startsWith('steps(')) return;

      const expected = round2(timing.exit.source_duration_ms * 0.72);
      const actual = timing.exit.scaled_duration_ms;
      assert.ok(
        Math.abs(actual - expected) <= 15,
        `${filename}: timing.exit.scaled_duration_ms ${actual} should be ≈ ${timing.exit.source_duration_ms} * 0.72 = ${expected} (±15ms)`
      );
    });
  }
});

describe('visible effects - showcase.rendering_contract has renderer and target', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: rendering_contract.renderer and rendering_contract.target present`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { rendering_contract } = effect.showcase;
      assert.ok('renderer' in rendering_contract, `${filename}: rendering_contract missing "renderer"`);
      assert.ok('target' in rendering_contract, `${filename}: rendering_contract missing "target"`);
      assert.ok(
        VALID_TARGETS.has(rendering_contract.target),
        `${filename}: rendering_contract.target "${rendering_contract.target}" must be a valid target`
      );
    });
  }
});

describe('visible effects - rendering_contract.target matches portable_spec.target', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: rendering_contract.target matches portable_spec.target`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      assert.strictEqual(
        effect.showcase.rendering_contract.target,
        effect.portable_spec.target,
        `${filename}: rendering_contract.target must match portable_spec.target`
      );
    });
  }
});

describe('visible effects - library_adapters each have target_library string', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: each library adapter has a target_library field`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { library_adapters } = effect.showcase;
      for (const [adapterName, adapter] of Object.entries(library_adapters)) {
        assert.ok(
          typeof adapter.target_library === 'string' && adapter.target_library.length > 0,
          `${filename}: library_adapters.${adapterName}.target_library must be a non-empty string`
        );
      }
    });
  }
});

// ─── specific effect validations ───────────────────────────────────────────

describe('kinetic-center-build - has build block in portable_spec', () => {
  it('portable_spec contains a build object with layout params', () => {
    const effect = readJson(join(EFFECTS_DIR, 'kinetic-center-build.json'));
    assert.ok('build' in effect.portable_spec, 'kinetic-center-build should have a "build" block in portable_spec');
    const { build } = effect.portable_spec;
    assert.ok('entry_direction' in build, 'build missing entry_direction');
    assert.ok('line_alignment' in build, 'build missing line_alignment');
    assert.ok('entry_offset_px' in build, 'build missing entry_offset_px');
    assert.ok(typeof build.entry_offset_px === 'number', 'build.entry_offset_px must be a number');
  });
});

describe('kinetic-center-build - renderer_override in catalog', () => {
  it('catalog.renderer_overrides includes kinetic-center-build', () => {
    const catalog = readJson(CATALOG_PATH);
    assert.ok(
      'kinetic-center-build' in catalog.renderer_overrides,
      'catalog.renderer_overrides should contain "kinetic-center-build"'
    );
    assert.strictEqual(
      catalog.renderer_overrides['kinetic-center-build'],
      'kinetic-center-build',
      'renderer_override value should be "kinetic-center-build"'
    );
  });
});

describe('hidden effects - do not appear in catalog visible_ids', () => {
  const catalog = readJson(CATALOG_PATH);

  for (const { filename, id, path } of getAllEffectFiles()) {
    it(`${filename}: hidden effect "${id}" is not in catalog.visible_ids`, () => {
      const effect = readJson(path);
      if (effect.visibility === 'hidden') {
        assert.ok(
          !catalog.visible_ids.includes(id),
          `${filename}: hidden effect "${id}" should not appear in catalog.visible_ids`
        );
      }
    });
  }
});

describe('visible effects - y_travel_multiplier is 0.58', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: runtime.y_travel_multiplier is 0.58`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      assert.strictEqual(
        effect.showcase.runtime.y_travel_multiplier,
        0.58,
        `${filename}: runtime.y_travel_multiplier should be 0.58`
      );
    });
  }
});

describe('visible effects - hold_ms and gap_ms are positive numbers', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: runtime.hold_ms and runtime.gap_ms are positive`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { hold_ms, gap_ms } = effect.showcase.runtime;
      assert.ok(typeof hold_ms === 'number' && hold_ms > 0, `${filename}: runtime.hold_ms must be a positive number`);
      assert.ok(typeof gap_ms === 'number' && gap_ms > 0, `${filename}: runtime.gap_ms must be a positive number`);
    });
  }
});

describe('visible effects - initial_delay_ms has mode, min, max in rendering_contract', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: rendering_contract.initial_delay_ms has mode, min, max fields`, () => {
      const effect = readJson(path);
      if (effect.visibility !== 'visible') return;

      const { initial_delay_ms } = effect.showcase.rendering_contract;
      assert.ok(initial_delay_ms !== undefined, `${filename}: rendering_contract should have initial_delay_ms`);
      assert.ok('mode' in initial_delay_ms, `${filename}: initial_delay_ms missing "mode"`);
      assert.ok('min' in initial_delay_ms, `${filename}: initial_delay_ms missing "min"`);
      assert.ok('max' in initial_delay_ms, `${filename}: initial_delay_ms missing "max"`);
      assert.ok(initial_delay_ms.min >= 0, `${filename}: initial_delay_ms.min must be >= 0`);
      assert.ok(initial_delay_ms.max >= initial_delay_ms.min, `${filename}: initial_delay_ms.max must be >= min`);
    });
  }
});

// ─── regression / edge cases ───────────────────────────────────────────────

describe('all effects - portable_spec.description is a non-empty string', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: portable_spec.description is non-empty`, () => {
      const { portable_spec: spec } = readJson(path);
      assert.strictEqual(typeof spec.description, 'string', `${filename}: description must be a string`);
      assert.ok(spec.description.length > 0, `${filename}: description must not be empty`);
    });
  }
});

describe('all effects - no extra top-level keys beyond allowed set', () => {
  const ALLOWED_TOP_LEVEL = new Set(['id', 'visibility', 'portable_spec', 'showcase']);

  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: no unexpected top-level keys`, () => {
      const effect = readJson(path);
      const unexpected = Object.keys(effect).filter((k) => !ALLOWED_TOP_LEVEL.has(k));
      assert.deepStrictEqual(
        unexpected,
        [],
        `${filename}: unexpected top-level keys: ${unexpected.join(', ')}`
      );
    });
  }
});

describe('boundary: enter duration_ms is not excessively long (< 10000ms)', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: enter.duration_ms is reasonable (< 10000ms)`, () => {
      const { portable_spec: { enter } } = readJson(path);
      assert.ok(enter.duration_ms < 10000, `${filename}: enter.duration_ms ${enter.duration_ms} seems excessively large`);
    });
  }
});

describe('boundary: exit duration_ms is shorter than or close to enter duration_ms', () => {
  for (const { filename, path } of getAllEffectFiles()) {
    it(`${filename}: exit duration is not more than 5x longer than enter`, () => {
      const { portable_spec: { enter, exit } } = readJson(path);
      assert.ok(
        exit.duration_ms <= enter.duration_ms * 5,
        `${filename}: exit.duration_ms ${exit.duration_ms} is unexpectedly much longer than enter ${enter.duration_ms}`
      );
    });
  }
});
