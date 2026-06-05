#!/usr/bin/env node

/**
 * Rook launcher - finds the platform-specific binary and executes it
 */

const { execSync, spawnSync } = require('node:child_process');
const path = require('node:path');
const os = require('node:os');

const PKG = '@dallay/rook';

function getPlatform() {
  const arch = os.arch() === 'arm64' ? 'arm64' : 'x64';
  const osPlatform = os.platform();
  
  let platform;
  if (osPlatform === 'win32') {
    platform = 'windows';
  } else if (osPlatform === 'darwin') {
    platform = 'darwin';
  } else {
    platform = 'linux';
  }
  
  return `${platform}-${arch}`;
}

function findBinary() {
  const osPlatform = os.platform();
  const exeSuffix = osPlatform === 'win32' ? '.exe' : '';
  
  // Try platform-specific optional dependency first
  const platformPkg = `${PKG}-${getPlatform()}`;
  try {
    const binaryPath = require.resolve(`${platformPkg}/bin/rook${exeSuffix}`);
    return binaryPath;
  } catch {
    // Fall back to PATH lookup
    const binaryName = `rook${exeSuffix}`;
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
}

try {
  const binaryPath = findBinary();
  const args = process.argv.slice(2);
  const result = spawnSync(binaryPath, args, {
    stdio: 'inherit',
    cwd: process.cwd()
  });
  
  if (result.error) {
    throw result.error;
  }
  
  // Handle signal termination: if status is null, the child was killed by a signal
  if (result.status !== null) {
    process.exit(result.status);
  } else if (result.signal) {
    // Exit with non-zero code to reflect signal termination
    // Convention: 128 + signal number, but we don't have the signal number here
    process.exit(1);
  } else {
    process.exit(0);
  }
} catch (error) {
  console.error('Rook error:', error.message);
  process.exit(1);
}