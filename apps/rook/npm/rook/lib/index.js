#!/usr/bin/env node

/**
 * Rook launcher - finds the platform-specific binary and executes it
 */

const { execSync } = require('node:child_process');
const path = require('node:path');
const os = require('node:os');

const PKG = '@dallay/rook';

function getPlatform() {
  const arch = os.arch() === 'arm64' ? 'arm64' : 'x64';
  let platform;
  if (os.platform() === 'win32') {
    platform = 'windows';
  } else if (os.platform() === 'darwin') {
    platform = 'darwin';
  } else {
    platform = 'linux';
  }
  return `${platform}-${arch}`;
}

function findBinary() {
  // Try platform-specific optional dependency first
  const platformPkg = `${PKG}-${getPlatform()}`;
  try {
    const binaryPath = require.resolve(`${platformPkg}/bin/rook${os.platform() === 'win32' ? '.exe' : ''}`);
    return binaryPath;
  } catch {
    // Fall back to PATH lookup
    const binaryName = `rook${os.platform() === 'win32' ? '.exe' : ''}`;
    try {
      const globalPath = execSync(`npm root -g`, { encoding: 'utf8' }).trim();
      const globalBinary = path.join(globalPath, platformPkg, 'bin', binaryName);
      require('node:fs').accessSync(globalBinary);
      return globalBinary;
    } catch {
      // Last resort: look in PATH
      try {
        return execSync(`which ${binaryName}`, { encoding: 'utf8' }).trim();
      } catch {
        console.error(`Error: Could not find rook binary. Install with: npm install -g ${PKG}`);
        process.exit(1);
      }
    }
  }
  return null;
}

try {
  const binaryPath = findBinary();
  const args = process.argv.slice(2);
  execSync(`"${binaryPath}" ${args.join(' ')}`, {
    stdio: 'inherit',
    cwd: process.cwd()
  });
} catch (error) {
  console.error('Rook error:', error.message);
  process.exit(1);
}