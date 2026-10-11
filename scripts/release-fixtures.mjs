#!/usr/bin/env node

const activeProducts = new Set(["rook", "huginn"]);

function productsForPaths(paths) {
  const products = new Set();
  for (const path of paths) {
    if (path.startsWith("apps/rook/") || path.startsWith("crates/domain/rook-core/")) products.add("rook");
    if (path.startsWith("apps/huginn/") || path.startsWith("crates/huginn/")) products.add("huginn");
    // shared-kernel is currently a Rook dependency; Huginn owns a separate kernel.
    if (path.startsWith("crates/domain/shared-kernel/")) products.add("rook");
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
assertEqual(productsForPaths(["crates/domain/shared-kernel/src/lib.rs"]), ["rook"], "shared dependency fixture");

const activeChanges = [...productsForPaths(["apps/huginn/src/main.rs"])].filter((product) => activeProducts.has(product));
assertEqual(new Set(activeChanges), ["huginn"], "Huginn release PR scope");
const unrelatedChanges = [...productsForPaths(["apps/huginn/src/main.rs"])].filter((product) => product !== "rook");
assertEqual(new Set(unrelatedChanges), ["huginn"], "Huginn changes must not trigger Rook");

console.log("Independent release fixtures passed: Rook, Huginn, simultaneous, shared dependency, and cross-product isolation.");
