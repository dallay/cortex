#!/usr/bin/env node
// Wrapper for vue-tsc — avoids path conflicts with npm_link_all_packages.
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const vueTscBin = join(
	__dirname,
	"node_modules",
	"vue-tsc",
	"bin",
	"vue-tsc.js",
);

const child = spawn("node", [vueTscBin, ...process.argv.slice(2)], {
	stdio: "inherit",
	cwd: __dirname,
});

child.on("close", (code) => {
	process.exit(code || 0);
});
