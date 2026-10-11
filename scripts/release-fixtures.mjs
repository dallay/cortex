#!/usr/bin/env node

import { readFile } from "node:fs/promises";

const config = JSON.parse(await readFile("release-please-config.json", "utf8"));
const products = Object.entries(config.packages ?? {}).map(([packagePath, settings]) => ({
  name: settings.component,
  prefixes: [`${packagePath}/`],
}));
const activeProducts = new Set(products.map(({ name }) => name));

function productsForPaths(paths) {
  const matched = new Set();
  for (const path of paths) {
    for (const product of products) {
      if (product.prefixes.some((prefix) => path.startsWith(prefix))) matched.add(product.name);
    }
  }
  return matched;
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
assertEqual(productsForPaths(["crates/domain/shared-kernel/src/lib.rs"]), [], "shared dependency outside product path fixture");

const huginnChanges = productsForPaths(["apps/huginn/src/main.rs"]);
assertEqual(new Set([...huginnChanges].filter((product) => activeProducts.has(product))), ["huginn"], "Huginn release PR scope");
assertEqual(new Set([...huginnChanges].filter((product) => product === "rook")), [], "Huginn changes must not trigger Rook");

console.log("Independent release fixtures passed: Rook, Huginn, simultaneous, shared dependency, and cross-product isolation.");
