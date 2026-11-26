const LOOPBACK_HOSTS = ["localhost", "127.0.0.1", "[::1]"];

/**
 * True when the dashboard runs in a browser on the Rook loopback host.
 *
 * Codex account linking uses a loopback redirect callback, so a remote
 * browser cannot complete the flow even when the operator enabled it.
 */
export function isLoopbackBrowser(
	hostname: string = window.location.hostname,
): boolean {
	return LOOPBACK_HOSTS.includes(hostname);
}
