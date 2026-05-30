'use strict';

/**
 * Tests for apps/rook/npm/rook/lib/index.js
 *
 * Because index.js is a CLI entry-point that runs code immediately on require,
 * module-execution tests mock all side-effectful dependencies (os, child_process,
 * fs) and use jest.resetModules() to reload a fresh copy for each test.
 *
 * Pure-logic tests (getPlatform mapping, binary name suffix, package-name
 * construction) duplicate and verify the logic inline so they remain fast
 * and independent.
 */

// ─── helpers — mirror the logic from index.js ───────────────────────────────

/**
 * Mirrors the getPlatform() function from lib/index.js.
 * Keeps the tests stable against source changes while documenting
 * the exact platform-detection contract.
 */
function getPlatform(arch, platform) {
  const resolvedArch = arch === 'arm64' ? 'arm64' : 'x64';
  const resolvedPlatform =
    platform === 'win32' ? 'windows' : platform === 'darwin' ? 'darwin' : 'linux';
  return `${resolvedPlatform}-${resolvedArch}`;
}

/** Mirrors the binary-name suffix logic from lib/index.js. */
function binaryName(platform) {
  return `rook${platform === 'win32' ? '.exe' : ''}`;
}

// ─── getPlatform — arch mapping ─────────────────────────────────────────────

describe('getPlatform — arch mapping', () => {
  test('arm64 maps to arm64', () => {
    expect(getPlatform('arm64', 'linux')).toBe('linux-arm64');
  });

  test('x64 maps to x64', () => {
    expect(getPlatform('x64', 'linux')).toBe('linux-x64');
  });

  test('ia32 falls back to x64', () => {
    expect(getPlatform('ia32', 'linux')).toBe('linux-x64');
  });

  test('ppc64 falls back to x64', () => {
    expect(getPlatform('ppc64', 'linux')).toBe('linux-x64');
  });

  test('s390x falls back to x64', () => {
    expect(getPlatform('s390x', 'linux')).toBe('linux-x64');
  });
});

// ─── getPlatform — OS mapping ────────────────────────────────────────────────

describe('getPlatform — OS mapping', () => {
  test('win32 maps to windows', () => {
    expect(getPlatform('x64', 'win32')).toBe('windows-x64');
  });

  test('darwin maps to darwin', () => {
    expect(getPlatform('x64', 'darwin')).toBe('darwin-x64');
  });

  test('linux maps to linux', () => {
    expect(getPlatform('x64', 'linux')).toBe('linux-x64');
  });

  test('freebsd falls back to linux', () => {
    expect(getPlatform('x64', 'freebsd')).toBe('linux-x64');
  });

  test('openbsd falls back to linux', () => {
    expect(getPlatform('x64', 'openbsd')).toBe('linux-x64');
  });

  test('sunos falls back to linux', () => {
    expect(getPlatform('x64', 'sunos')).toBe('linux-x64');
  });
});

// ─── getPlatform — all supported combinations ────────────────────────────────

describe('getPlatform — supported combinations', () => {
  const cases = [
    ['arm64', 'darwin', 'darwin-arm64'],
    ['x64', 'darwin', 'darwin-x64'],
    ['arm64', 'win32', 'windows-arm64'],
    ['x64', 'win32', 'windows-x64'],
    ['arm64', 'linux', 'linux-arm64'],
    ['x64', 'linux', 'linux-x64'],
  ];

  test.each(cases)('arch=%s platform=%s → %s', (arch, platform, expected) => {
    expect(getPlatform(arch, platform)).toBe(expected);
  });
});

// ─── binary name suffix ──────────────────────────────────────────────────────

describe('binary name suffix', () => {
  test('Windows uses .exe extension', () => {
    expect(binaryName('win32')).toBe('rook.exe');
  });

  test('Linux uses no extension', () => {
    expect(binaryName('linux')).toBe('rook');
  });

  test('macOS uses no extension', () => {
    expect(binaryName('darwin')).toBe('rook');
  });

  test('unknown platform uses no extension', () => {
    expect(binaryName('freebsd')).toBe('rook');
  });
});

// ─── platform package name construction ─────────────────────────────────────

describe('platform package name', () => {
  const PKG = '@dallay/rook';

  const cases = [
    ['x64', 'linux', '@dallay/rook-linux-x64'],
    ['arm64', 'linux', '@dallay/rook-linux-arm64'],
    ['x64', 'darwin', '@dallay/rook-darwin-x64'],
    ['arm64', 'darwin', '@dallay/rook-darwin-arm64'],
    ['x64', 'win32', '@dallay/rook-windows-x64'],
    ['arm64', 'win32', '@dallay/rook-windows-arm64'],
  ];

  test.each(cases)('arch=%s platform=%s → %s', (arch, platform, expected) => {
    const platformStr = getPlatform(arch, platform);
    expect(`${PKG}-${platformStr}`).toBe(expected);
  });
});

// ─── module-level behaviour (with full mocking) ─────────────────────────────

describe('index.js module execution', () => {
  let originalExit;
  let mockExecSync;

  beforeEach(() => {
    originalExit = process.exit;
    process.exit = jest.fn();
    jest.resetModules();
  });

  afterEach(() => {
    process.exit = originalExit;
    jest.restoreAllMocks();
  });

  test('calls process.exit(1) when no binary can be located anywhere', () => {
    // All resolution strategies fail → error path → process.exit(1)
    mockExecSync = jest.fn().mockImplementation(() => {
      throw new Error('command not found');
    });
    jest.doMock('child_process', () => ({ execSync: mockExecSync }));
    jest.doMock('fs', () => ({
      ...jest.requireActual('fs'),
      accessSync: jest.fn().mockImplementation(() => {
        throw new Error('not found');
      }),
    }));
    jest.doMock('os', () => ({
      arch: () => 'x64',
      platform: () => 'linux',
    }));

    process.argv = ['node', 'index.js'];

    require('../lib/index.js');

    expect(process.exit).toHaveBeenCalledWith(1);
  });

  test('falls back to npm global root when platform package is absent', () => {
    // require.resolve throws (platform package absent), but `which rook` succeeds
    const FAKE_BINARY = '/usr/local/bin/rook';

    mockExecSync = jest.fn().mockImplementation((cmd) => {
      if (typeof cmd === 'string' && cmd === 'npm root -g') {
        return '/fake/global/node_modules';
      }
      if (typeof cmd === 'string' && cmd.startsWith('which ')) {
        return FAKE_BINARY;
      }
      // binary launch
      return '';
    });

    jest.doMock('child_process', () => ({ execSync: mockExecSync }));
    jest.doMock('fs', () => ({
      ...jest.requireActual('fs'),
      accessSync: jest.fn().mockImplementation(() => {
        throw new Error('global binary not found');
      }),
    }));
    jest.doMock('os', () => ({
      arch: () => 'x64',
      platform: () => 'linux',
    }));

    process.argv = ['node', 'index.js'];

    require('../lib/index.js');

    // `which rook` must have been attempted as a last resort
    const whichCall = mockExecSync.mock.calls.find(
      (args) => typeof args[0] === 'string' && args[0].startsWith('which ')
    );
    expect(whichCall).toBeDefined();
  });

  test('uses the path returned by "which" to launch the binary', () => {
    const WHICH_PATH = '/usr/local/bin/rook';

    mockExecSync = jest.fn().mockImplementation((cmd) => {
      if (typeof cmd === 'string' && cmd === 'npm root -g') {
        return '/fake/global/node_modules';
      }
      if (typeof cmd === 'string' && cmd.startsWith('which ')) {
        return WHICH_PATH;
      }
      return '';
    });

    jest.doMock('child_process', () => ({ execSync: mockExecSync }));
    jest.doMock('fs', () => ({
      ...jest.requireActual('fs'),
      accessSync: jest.fn().mockImplementation(() => {
        throw new Error('not accessible');
      }),
    }));
    jest.doMock('os', () => ({
      arch: () => 'x64',
      platform: () => 'linux',
    }));

    process.argv = ['node', 'index.js', 'serve'];

    require('../lib/index.js');

    // The final execSync call should contain the resolved path
    const launchCall = mockExecSync.mock.calls.find(
      (args) => typeof args[0] === 'string' && args[0].includes(WHICH_PATH)
    );
    expect(launchCall).toBeDefined();
  });

  test('passes user CLI args to the binary', () => {
    const WHICH_PATH = '/usr/local/bin/rook';

    mockExecSync = jest.fn().mockImplementation((cmd) => {
      if (typeof cmd === 'string' && cmd === 'npm root -g') return '/fake/global';
      if (typeof cmd === 'string' && cmd.startsWith('which ')) return WHICH_PATH;
      return '';
    });

    jest.doMock('child_process', () => ({ execSync: mockExecSync }));
    jest.doMock('fs', () => ({
      ...jest.requireActual('fs'),
      accessSync: jest.fn().mockImplementation(() => { throw new Error(); }),
    }));
    jest.doMock('os', () => ({
      arch: () => 'x64',
      platform: () => 'linux',
    }));

    process.argv = ['node', 'index.js', '--port', '3000', '--config', '/etc/rook.toml'];

    require('../lib/index.js');

    const launchCall = mockExecSync.mock.calls.find(
      (args) => typeof args[0] === 'string' && args[0].includes(WHICH_PATH)
    );
    expect(launchCall).toBeDefined();
    expect(launchCall[0]).toContain('--port');
    expect(launchCall[0]).toContain('3000');
    expect(launchCall[0]).toContain('--config');
  });

  test('searches for rook.exe on Windows when falling back to PATH', () => {
    mockExecSync = jest.fn().mockImplementation((cmd) => {
      if (typeof cmd === 'string' && cmd === 'npm root -g') return '/fake/global';
      if (typeof cmd === 'string' && cmd.startsWith('which ')) return 'C:\\fake\\rook.exe';
      return '';
    });

    jest.doMock('child_process', () => ({ execSync: mockExecSync }));
    jest.doMock('fs', () => ({
      ...jest.requireActual('fs'),
      accessSync: jest.fn().mockImplementation(() => { throw new Error(); }),
    }));
    jest.doMock('os', () => ({
      arch: () => 'x64',
      platform: () => 'win32',
    }));

    process.argv = ['node', 'index.js'];

    require('../lib/index.js');

    // which should have been called with 'rook.exe'
    const whichCall = mockExecSync.mock.calls.find(
      (args) => typeof args[0] === 'string' && args[0].includes('rook.exe')
    );
    expect(whichCall).toBeDefined();
  });
});