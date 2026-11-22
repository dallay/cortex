'use strict';

const {spawnSync} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const GLOBAL_PATHS = [
  /^Cargo\.lock$/,
  /^Cargo\.toml$/,
  /^rust-toolchain(?:\.toml)?$/,
  /^\.cargo(?:\/|$)/,
  /^\.github\/workflows(?:\/|$)/,
  /^\.github\/ci-path-filters\.json$/,
  /^\.github\/scripts\/ci-[^/]+\.cjs$/,
  /^\.github\/scripts\/ci-[^/]+\.test\.cjs$/,
  /^\.config\/nextest\.toml$/,
  /^\.bazel(?:ignore|rc|version)$/,
  /^MODULE\.bazel(?:\.lock)?$/,
  /(^|\/)BUILD\.bazel$/,
  /^dev\/bazel(?:\/|$)/,
];

function isMarkdownPath(changed) {
  return /\.mdx?$/.test(changed);
}

function isDocumentationPath(changed) {
  return (changed.startsWith('docs/') && /\.md$/.test(changed))
      || /^\.agents\/.+\.md$/.test(changed)
      || /^[^/]+\.md$/.test(changed);
}

function fallback(reason) {
  return {mode: 'workspace', packages: ['workspace'], reason};
}

function parseMetadata(raw) {
  const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
  if (!value || !Array.isArray(value.packages) || !Array.isArray(value.workspace_members)
      || !value.resolve || !Array.isArray(value.resolve.nodes)) {
    throw new Error('cargo metadata has an invalid packages/workspace_members/resolve shape');
  }
  const packages = new Map();
  for (const pkg of value.packages) {
    if (!pkg || typeof pkg.id !== 'string' || typeof pkg.name !== 'string' || typeof pkg.manifest_path !== 'string' || !Array.isArray(pkg.dependencies)) {
      throw new Error('cargo metadata contains an invalid package entry');
    }
    const root = path.posix.dirname(pkg.manifest_path.replaceAll('\\', '/'));
    packages.set(pkg.id, {id: pkg.id, name: pkg.name, root, dependencies: pkg.dependencies});
  }
  const members = new Set(value.workspace_members);
  if (!members.size || [...members].some((id) => !packages.has(id))) {
    throw new Error('cargo metadata has missing or invalid workspace members');
  }
  const resolved = new Map();
  for (const node of value.resolve.nodes) {
    if (!node || typeof node.id !== 'string' || !Array.isArray(node.deps)) {
      throw new Error('cargo metadata contains an invalid resolved dependency node');
    }
    const edges = new Set();
    for (const dep of node.deps) {
      if (!dep || typeof dep.pkg !== 'string' || !Array.isArray(dep.dep_kinds)) {
        throw new Error(`cargo metadata contains an invalid resolved dependency in ${node.id}`);
      }
      if (!packages.has(dep.pkg)) throw new Error(`resolved dependency package is missing: ${dep.pkg}`);
      for (const kind of dep.dep_kinds) {
        if (!kind || !(kind.kind === null || kind.kind === 'normal' || kind.kind === 'build' || kind.kind === 'dev')) {
          throw new Error(`cargo metadata contains an invalid dependency kind in ${node.id}`);
        }
        if (kind.kind === null || kind.kind === 'normal' || kind.kind === 'build' || kind.kind === 'dev') edges.add(dep.pkg);
      }
    }
    resolved.set(node.id, edges);
  }
  if ([...members].some((id) => !resolved.has(id))) throw new Error('cargo metadata has missing workspace dependency nodes');
  return {packages, members, resolved};
}

function resolveAffected(changedPaths, rawMetadata, repoRoot = '.') {
  try {
    if (!Array.isArray(changedPaths) || changedPaths.length === 0) {
      return fallback('no changed paths were provided');
    }
    const normalizedPaths = [];
    for (const input of changedPaths) {
      if (typeof input !== 'string' || input.length === 0 || input.includes('\0') || input.startsWith('/') || /^[A-Za-z]:/.test(input)) {
        return fallback(`ambiguous changed path: ${String(input)}`);
      }
      const changed = path.posix.normalize(input.replaceAll('\\', '/'));
      if (changed === '..' || changed.startsWith('../')) {
        return fallback(`changed path escapes the repository: ${input}`);
      }
      if (GLOBAL_PATHS.some((pattern) => pattern.test(changed))) {
        return fallback(`global path changed: ${changed}`);
      }
      if (isMarkdownPath(changed) && !isDocumentationPath(changed)) {
        return fallback(`Markdown outside documentation paths: ${changed}`);
      }
      normalizedPaths.push(changed);
    }
    if (normalizedPaths.every(isDocumentationPath)) {
      return {mode: 'docs-only', packages: [], reason: 'documentation-only changes do not select Rust crates'};
    }
    const metadata = parseMetadata(rawMetadata);
    const root = path.resolve(repoRoot).replaceAll('\\', '/');
    const memberPackages = [...metadata.members].map((id) => metadata.packages.get(id)).filter((pkg) => pkg.root.startsWith(`${root}/`) || pkg.root === root);
    const roots = memberPackages.map((pkg) => ({
      ...pkg,
      relativeRoot: path.posix.relative(root, pkg.root),
    }));
    if (roots.some((pkg) => pkg.relativeRoot.startsWith('../') || path.posix.isAbsolute(pkg.relativeRoot))) {
      throw new Error('workspace package manifest is outside repository root');
    }
    const direct = new Set();
    for (const changed of normalizedPaths) {
      if (isDocumentationPath(changed)) continue;
      const owners = roots.filter((pkg) => pkg.relativeRoot !== ''
          && (changed === pkg.relativeRoot || changed.startsWith(`${pkg.relativeRoot}/`)));
      if (changed === 'Cargo.toml' || /(^|\/)Cargo\.toml$/.test(changed)) {
        return fallback(`package manifest changed: ${changed}`);
      }
      if (!owners.length) return fallback(`unmapped changed path: ${changed}`);
      const deepest = Math.max(...owners.map((pkg) => pkg.relativeRoot.length));
      const nearest = owners.filter((pkg) => pkg.relativeRoot.length === deepest);
      if (nearest.length !== 1) return fallback(`ambiguous package ownership: ${changed}`);
      direct.add(nearest[0].id);
    }
    const affected = new Set(direct);
    let changed = true;
    while (changed) {
      changed = false;
      for (const pkg of memberPackages) {
        if (affected.has(pkg.id)) continue;
        const edges = metadata.resolved.get(pkg.id);
        if (!edges) throw new Error(`missing resolved dependency node for ${pkg.id}`);
        for (const dependencyId of edges) {
          if (affected.has(dependencyId)) {
            affected.add(pkg.id);
            changed = true;
            break;
          }
        }
      }
    }
    const names = [...affected].map((id) => metadata.packages.get(id).name);
    if (names.some((name) => typeof name !== 'string')) throw new Error('workspace package has no name');
    names.sort();
    return {mode: 'packages', packages: names, reason: 'changed package paths and reverse dependency closure'};
  } catch (error) {
    return fallback(`resolver error: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function run(command, args, cwd) {
  const result = spawnSync(command, args, {cwd, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024});
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited ${result.status}: ${(result.stderr || '').trim()}`);
  return result.stdout;
}

function parseNameStatusDiff(output) {
  if (typeof output !== 'string' || (output.length > 0 && !output.endsWith('\0'))) {
    throw new Error('malformed git diff name-status output');
  }
  const fields = output.length === 0 ? [] : output.slice(0, -1).split('\0');
  const paths = [];
  for (let i = 0; i < fields.length;) {
    const status = fields[i++];
    if (/^[RC](?:[0-9]{1,3})?$/.test(status)) {
      if (i + 1 >= fields.length || !fields[i] || !fields[i + 1]) {
        throw new Error('incomplete rename/copy record in git diff');
      }
      paths.push(fields[i++], fields[i++]);
      continue;
    }
    if (!/^[ADMTUXB]$/.test(status)) throw new Error(`invalid status record in git diff: ${status}`);
    if (i >= fields.length || !fields[i]) throw new Error('incomplete path record in git diff');
    paths.push(fields[i++]);
  }
  return paths;
}

function changedPathsFromDiff(base, head, cwd) {
  if (!/^[0-9a-f]{40,64}$/i.test(base) || !/^[0-9a-f]{40,64}$/i.test(head)) {
    throw new Error('base/head must be full hexadecimal Git object IDs');
  }
  const output = run('git', ['diff', '--name-status', '-z', '--find-renames', `${base}...${head}`], cwd);
  return parseNameStatusDiff(output);
}

function main() {
  const cwd = process.cwd();
  let plan;
  try {
    const {BASE: base, HEAD: head} = process.env;
    const paths = changedPathsFromDiff(base, head, cwd);
    plan = resolveAffected(paths, null, cwd);
    if (plan.mode !== 'docs-only') {
      const metadata = run('cargo', ['metadata', '--format-version', '1', '--all-features'], cwd);
      plan = resolveAffected(paths, metadata, cwd);
    }
  } catch (error) {
    plan = fallback(`resolver error: ${error instanceof Error ? error.message : String(error)}`);
  }
  const output = process.env.GITHUB_OUTPUT;
  if (output) {
    try {
      fs.appendFileSync(output, `mode=${plan.mode}\n`);
    } catch (error) {
      process.stderr.write(`Could not append affected-crate mode to job outputs: ${error.message}\n`);
    }
  }
  const summary = process.env.GITHUB_STEP_SUMMARY;
  const markdown = `## Affected crates (shadow mode)\n\n- **Plan:** ${plan.packages.join(', ')}\n- **Mode:** ${plan.mode}\n- **Reason:** ${plan.reason.replace(/[\r\n]/g, ' ')}\n\nThis plan is informational. Existing workspace checks remain unchanged.\n`;
  if (summary) {
    try {
      fs.appendFileSync(summary, markdown);
    } catch (error) {
      process.stderr.write(`Could not append affected-crate plan to job summary: ${error.message}\n`);
    }
  }
  process.stdout.write(markdown);
}

if (require.main === module) main();
module.exports = {changedPathsFromDiff, parseMetadata, parseNameStatusDiff, resolveAffected};
