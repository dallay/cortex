import { describe, expect, it } from "vitest";
import { isLoopbackBrowser } from "./browserHost";

describe("isLoopbackBrowser", () => {
	it.each(["localhost", "127.0.0.1", "[::1]"])(
		"accepts the loopback host %s",
		(hostname) => {
			expect(isLoopbackBrowser(hostname)).toBe(true);
		},
	);

	it.each(["example.com", "192.168.1.10", "rook.internal", ""])(
		"rejects the non-loopback host %s",
		(hostname) => {
			expect(isLoopbackBrowser(hostname)).toBe(false);
		},
	);
});
