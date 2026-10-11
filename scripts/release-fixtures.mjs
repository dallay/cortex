#!/usr/bin/env node

import { readFile } from "node:fs/promises";

const config = JSON.parse(await readFile("release-please-config.json", "utf8"));
const packages = Object.entries(config.packages);
const activeProducts = new Set(packages.map(([, pkg]) => pkg.component));

function productsForPaths(paths) {
  const products = new Set();
  for (const path of paths) {
    for (const [packagePath, pkg] of packages) {
      if (path.startsWith(`${packagePath}/`)) products.add(pkg.component);
    }
  }
  return products;
}

function assertEqual(actual, expected, label) {
  const received = [...actual].sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(received) !== JSON.stringify(wanted)) {
    throw new Error(`${label}: expected ${wanted.join(",") || "<none>"}, received ${received.join(",") || "<none>"}`);
  }
}

assertEqual(productsForPaths(["apps/rook/src/main.rs"]), ["rook"], "Rook-only fixture");
assertEqual(productsForPaths(["apps/huginn/src/main.rs"]), ["huginn"], "Huginn-only fixture");
assertEqual(productsForPaths(["apps/rook/src/main.rs", "apps/huginn/src/main.rs"]), ["huginn", "rook"], "simultaneous fixture");
// These fixtures cover configured package paths, not cargo-workspace dependency propagation.
assertEqual(productsForPaths(["crates/domain/shared-kernel/src/lib.rs"]), [], "path outside configured packages");
assertEqual(productsForPaths(["apps/rook-other/src/main.rs"]), [], "package prefix boundary");

const activeChanges = [...productsForPaths(["apps/huginn/src/main.rs"])].filter((product) => activeProducts.has(product));
assertEqual(new Set(activeChanges), ["huginn"], "Huginn release PR scope");
const unrelatedChanges = [...productsForPaths(["apps/huginn/src/main.rs"])].filter((product) => product !== "rook");
assertEqual(new Set(unrelatedChanges), ["huginn"], "Huginn changes must not trigger Rook");

console.log("Independent release fixtures passed: Rook, Huginn, simultaneous, unconfigured paths, and cross-product isolation.");
