---
name: best-practices
description: >-
  Security, compatibility, quality and performance rules for Cortex web surfaces,
  especially the Rook dashboard. Use for security audits, modernization, code
  quality reviews, or vulnerability checks.
license: MIT
metadata:
  author: cortex
  version: "2.0"
---

# Best practices

Rules for web code in this monorepo. They follow the priority order in
`.agents/AGENTS.md`: security, then performance, then UX. Apply them to
`apps/rook/dashboard` and any future web surface; Rust services follow the
security rules for headers, cookies and dependencies too.

## Security

### HTTPS and transport

- Serve production over HTTPS only. Do not ship `http://` assets, fonts, scripts
  or API calls that would be blocked as mixed content.
- Write absolute `https://` URLs. Protocol-relative `//host/path` inherits the
  page protocol; it is not a guarantee of HTTPS and is best avoided.
- Redirect HTTP to HTTPS at the edge and send HSTS:

```http
Strict-Transport-Security: max-age=31536000; includeSubDomains; preload
```

Only add `preload` once every subdomain is HTTPS-ready; removal is slow.

### Content Security Policy

Send CSP as an HTTP response header. A `<meta http-equiv>` tag works for basic
directives but cannot enforce `frame-ancestors`, `report-uri` or `sandbox`.

```http
Content-Security-Policy:
  default-src 'self';
  script-src 'self' 'nonce-<random>';
  style-src 'self' 'nonce-<random>';
  img-src 'self' data: https:;
  connect-src 'self' https://api.example.com;
  frame-ancestors 'none';
  base-uri 'self';
  form-action 'self';
  object-src 'none';
```

- Generate a fresh nonce per response; never reuse a literal nonce.
- Avoid `'unsafe-inline'` and `'unsafe-eval'`. Vue's runtime compiler needs
  `'unsafe-eval'`; the dashboard uses precompiled SFCs, so it does not.
- Start with `Content-Security-Policy-Report-Only` to find violations, then
  enforce.
- Do not add a CSP that the application cannot satisfy; a broken page is not a
  security win.

### Security headers

```http
# Clickjacking — CSP frame-ancestors is the modern control
X-Frame-Options: DENY

# Stop MIME sniffing
X-Content-Type-Options: nosniff

# Limit referrer leakage
Referrer-Policy: strict-origin-when-cross-origin

# Disable powerful features by default
Permissions-Policy: geolocation=(), microphone=(), camera=()
```

- **Do not send `X-XSS-Protection`.** It is obsolete, can introduce
  vulnerabilities in old browsers, and CSP replaces it.
- `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy` are useful when
  the app needs cross-origin isolation; add them deliberately, not by default.

### Dependencies and supply chain

Cortex uses **pnpm** and Cargo. Do not introduce npm/yarn lockfiles or scripts.

```bash
pnpm audit --audit-level=moderate     # JS dependencies
cargo audit                           # Rust dependencies (just ci-audit)
pnpm outdated -r                      # review upgrades
```

- CI already blocks on gitleaks, semgrep, trivy and `cargo audit`. Keep those
  gates; do not weaken them to land a change.
- Commit `pnpm-lock.yaml`; the dashboard must not contain its own lockfile.
- Review new dependencies for maintenance, license, install scripts and transitive
  weight. Prefer the platform API over a package.
- **Never load scripts from `polyfill.io`.** The domain was sold and served
  malicious code in 2024. Ship only code you build.

### Untrusted input

Validate at the trust boundary, then use typed values internally:

```ts
import { z } from "zod";

const ProviderSchema = z.object({
	name: z.string().min(1).max(120),
	baseUrl: z.string().url().optional(),
});

const provider = ProviderSchema.parse(await response.json());
```

```rust
// Rust: deserialize into a typed struct; reject unknown shapes.
let request: CreateProviderRequest = serde_json::from_slice(&body)?;
```

- Treat HTTP input, provider responses, model output, tool results and files as
  untrusted.
- Avoid `Object.assign(target, userInput)` and deep-merging untrusted objects
  into prototypes. Use an allowlist of fields or a schema.
- `JSON.parse(JSON.stringify(x))` is not a sanitizer; it drops `Date`, `Map`,
  functions and `undefined`, and does not stop prototype pollution.

### Rendering untrusted content

```vue
<!-- ❌ Raw HTML from an API or user -->
<div v-html="untrusted" />

<!-- ✅ Text is escaped by Vue -->
<div>{{ untrusted }}</div>
```

- Prefer text interpolation, `textContent` and framework escaping.
- `v-html` / `innerHTML` require sanitization with a reviewed library; none is
  installed in the dashboard, so do not add a raw-HTML path casually.
- Never interpolate untrusted data into `eval`, `new Function`, `document.write`
  or inline event handler attributes.

### Cookies and sessions

```http
Set-Cookie: session=<opaque-token>; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600
```

- `HttpOnly` keeps tokens away from JavaScript; `Secure` requires HTTPS;
  `SameSite=Strict` or `Lax` blocks most CSRF.
- The dashboard uses a double-submit CSRF token: state-changing requests send
  `X-CSRF-Token` matching the `csrf_token` cookie. Keep that logic in
  `src/lib/api.ts`; never bypass or duplicate it.
- Do not store credentials or tokens in `localStorage` when a cookie works.
- Never log cookies, authorization headers or API keys.

### Secrets

- Secrets belong in environment variables or the secret store, never in source,
  tests, fixtures or logs.
- Use placeholder values in examples; dev defaults are not production secrets.
- Rotate any credential that appears in a commit, a screenshot or a trace.

## Browser compatibility

### Document basics

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Page</title>
</head>
```

- Charset first, then viewport, then title.
- Set the document language; the dashboard ships `en` and `es` locales.

### Feature detection

```javascript
if ("IntersectionObserver" in window) {
  // use it
} else {
  // fallback
}
```

Do not branch on `navigator.userAgent`. Use feature detection or `@supports`:

```css
@supports (display: grid) {
  .container { display: grid; }
}

@supports not (display: grid) {
  .container { display: flex; }
}
```

### Targets

The dashboard uses Vite 8 with a modern baseline. Check `vite.config.ts` and the
project `browserslist`/build target before adding legacy polyfills. Do not add
`core-js` or a CDN polyfill for a feature the target browsers support.

## Code quality

- Format and lint with **Biome**: `pnpm --filter @dallay/rook-dashboard run lint`.
  Do not add ESLint, Prettier or a second formatter.
- Keep TypeScript strict. Avoid `any`; use `unknown` plus narrowing or a schema.
- Remove `console.log` from production paths. Log through the application's
  error reporting, with context and without secrets.
- Handle errors where they can be acted on. Surface a user-readable message and
  keep the technical detail in logs.

### Vue error handling

```ts
// main.ts
app.config.errorHandler = (error, instance, info) => {
	reportError(error, { component: instance?.$options.name, info });
};
```

```vue
<script setup lang="ts">
import { onErrorCaptured } from "vue";

onErrorCaptured((error) => {
	reportError(error);
	return false; // stop propagation after handling
});
</script>
```

- Every view needs loading, empty, error and success states (`LoadingState`,
  `EmptyState`, `ErrorBanner`).
- Clean up listeners, intervals and observers in `onUnmounted`; prefer
  `AbortController` when a request can outlive the component.

## Performance

### Loading

- Use `defer` for scripts that do not need to block parsing; use `type="module"`
  in modern builds.
- Preload the critical font or hero image; lazy-load below-the-fold images with
  `loading="lazy"` and explicit `width`/`height` to avoid layout shift.
- Import above-the-fold components eagerly and route-level views lazily.
- Do not ship a large dependency for a small utility.

### Runtime

```javascript
// ❌ One listener per item
items.forEach((item) => item.addEventListener("click", handleClick));

// ✅ Event delegation
container.addEventListener("click", (event) => {
	const target = event.target.closest(".item");
	if (target) handleClick(target);
});
```

```javascript
// ❌ Listener that outlives the component
window.addEventListener("resize", handler);

// ✅ Remove it, or tie it to an AbortSignal
const controller = new AbortController();
window.addEventListener("resize", handler, { signal: controller.signal });
// later
controller.abort();
```

- Use `computed` for derived state instead of recalculating in templates.
- Keep reactivity shallow for large lists (`shallowRef`, `markRaw` where apt).
- Batch DOM reads and writes; avoid layout thrash in loops.
- Do not add `waitForTimeout` or polling sleeps to fix a race. Fix the race.

## Tooling

| Task | Command |
|---|---|
| Lint/format | `pnpm --filter @dallay/rook-dashboard run lint` |
| Typecheck | `pnpm --filter @dallay/rook-dashboard run typecheck` |
| Unit tests | `pnpm --filter @dallay/rook-dashboard exec vitest run` |
| E2E | `just test-e2e` (Docker) |
| Rust audit | `just ci-audit` |
| Skills/Markdown | `just ci-lint` |

Use `just --list` for the canonical recipes. Do not add npm, yarn or Makefile
instructions to documentation or CI.

## Audit checklist

### Security

- [ ] HTTPS only; no mixed content
- [ ] CSP sent as a header; no `'unsafe-inline'`/`'unsafe-eval'` without a reason
- [ ] `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy` present
- [ ] No `X-XSS-Protection`
- [ ] Cookies use `Secure`, `HttpOnly`, `SameSite`
- [ ] CSRF token enforced on state-changing requests
- [ ] No secrets in source, logs, traces or fixtures
- [ ] `pnpm audit` and `cargo audit` clean; no `polyfill.io`
- [ ] Untrusted input validated; no raw `v-html` without sanitization

### Compatibility

- [ ] HTML5 doctype, charset first, viewport present, `lang` set
- [ ] Feature detection instead of user-agent branching
- [ ] No deprecated APIs (`document.write`, sync XHR, AppCache)
- [ ] Passive listeners for scroll/touch where they do not call `preventDefault`

### Quality and performance

- [ ] Biome passes; TypeScript strict; no `any` escapes
- [ ] No console noise; errors handled and reported
- [ ] Listeners, timers and observers cleaned up
- [ ] Images have dimensions and lazy loading where appropriate
- [ ] Loading, empty, error and success states covered
- [ ] No artificial sleeps, retries or `force: true` hiding a defect

## References

- [MDN Web Security](https://developer.mozilla.org/en-US/docs/Web/Security)
- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
- [OWASP Secure Headers](https://owasp.org/www-project-secure-headers/)
- [Vite build options](https://vite.dev/config/build-options)
- [Vue security](https://vuejs.org/guide/best-practices/security)
- [references/REFERENCE.md](references/REFERENCE.md) for HTML, privacy and
  permission details.
