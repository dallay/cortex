#!/usr/bin/env node
// Wrapper for Vite build — uses the programmatic API to avoid path conflicts
// between npm_link_all_packages and js_binary entry points.
import { build } from "vite";

await build();
