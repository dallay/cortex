#!/usr/bin/env node
// Wrapper for Vitest — uses the programmatic API to avoid path conflicts
// between npm_link_all_packages and js_test entry points.
import { startVitest } from "vitest/node";

const vitest = await startVitest(undefined, ["run"], {
	root: ".",
	coverage: { enabled: false },
});
if (!vitest) {
	process.exit(1);
}
const result = await vitest.exit();
process.exit(result ? 1 : 0);
