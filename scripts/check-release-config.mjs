#!/usr/bin/env node

import { readFile } from "node:fs/promises";

const config = JSON.parse(await readFile("release-please-config.json", "utf8"));
const manifest = JSON.parse(await readFile(".release-please-manifest.json", "utf8"));
const rook = config.packages?.["apps/rook"];
const huginn = config.packages?.["apps/huginn"];
const failures = [];

function assert(condition, message) {
  if (!condition) failures.push(message);
}

assert(config["separate-pull-requests"] === true, "separate-pull-requests must be true");
assert(config["always-update"] === true, "always-update must be true");
assert(config["include-component-in-tag"] === true, "component tags must be enabled");
assert(config["tag-separator"] === "@", "component tags must use @ as separator");
assert(Object.keys(manifest).length === 2, "active manifest must track Rook and Huginn independently");
assert(Object.keys(config.packages ?? {}).sort().join(",") === "apps/huginn,apps/rook", "active config must track Rook and Huginn independently");
assert(Boolean(rook), "apps/rook must be configured");
assert(Boolean(huginn), "apps/huginn must be configured");
assert(rook?.component === "rook", "apps/rook must use the rook component identity");
assert(huginn?.component === "huginn", "apps/huginn must use the huginn component identity");
assert(rook?.["changelog-path"] === "CHANGELOG.md", "Rook must own apps/rook/CHANGELOG.md");
assert(huginn?.["changelog-path"] === "CHANGELOG.md", "Huginn must own apps/huginn/CHANGELOG.md");
assert(manifest["apps/rook"] === "0.0.1", "active manifest must preserve Rook's current version");

const huginnCargo = await readFile("apps/huginn/Cargo.toml", "utf8");
assert(huginnCargo.includes('name = "huginn"'), "Huginn release requires canonical apps/huginn package identity");
const huginnVersion = huginnCargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
assert(manifest["apps/huginn"] === huginnVersion, "Huginn manifest baseline must match its app crate version");

const packageJson = JSON.parse(await readFile("apps/rook/npm/rook/package.json", "utf8"));
const expectedPlatforms = [
  "darwin-arm64",
  "darwin-x64",
  "linux-x64",
  "windows-x64",
];

assert(packageJson.name === "@dallay/rook", "the base npm package must be @dallay/rook");
assert(!Object.keys(packageJson.optionalDependencies ?? {}).some((name) => name.includes("`")), "npm dependency names must not contain backticks");
for (const platform of expectedPlatforms) {
  const name = `@dallay/rook-${platform}`;
  assert(packageJson.optionalDependencies?.[name] === packageJson.version, `${name} must match the base version`);
}

const workflow = await readFile(".github/workflows/release.yml", "utf8");
assert(workflow.includes("apps/rook--release_created"), "workflow must use the path-scoped Rook release output");
assert(workflow.includes("apps/huginn--release_created"), "workflow must expose the path-scoped Huginn release output");
assert(workflow.includes("apps/huginn--tag_name"), "workflow must expose the path-scoped Huginn tag output");
assert(workflow.includes("apps/huginn--sha"), "workflow must expose the path-scoped Huginn SHA output");
assert(workflow.includes("if: ${{ needs.release-please.outputs.rook_release_created == 'true' }}"), "binary build must remain Rook-only");
assert(!workflow.includes("release-please-config-huginn-candidate.json"), "workflow must not load the stale candidate config");
assert(!workflow.includes(".release-please-manifest-huginn-candidate.json"), "workflow must not load the stale candidate manifest");
assert(!workflow.includes("needs.release-please.outputs.new_release_created"), "workflow must not use root release outputs");
assert(!workflow.includes("cargo publish"), "Cargo publication must remain deferred for Rook GA");
assert(!workflow.includes("huginn_release_created == 'true'"), "Huginn must not trigger distribution jobs yet");
assert(!workflow.includes("ghcr.io/${{ github.repository }}"), "GHCR image must be product-scoped");

if (failures.length > 0) {
  console.error("Release configuration validation failed:");
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log("Independent Rook and Huginn release configuration validation passed; public channel jobs remain scoped to Rook.");
