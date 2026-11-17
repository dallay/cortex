# Artifact transfer boundaries

`artifact_route` authorizes URL state transitions. Its `http::ArtifactHttp` child
now performs bounded acquisition and returns verified owned bytes. Neither type
exposes a preparation service or executes downloaded model content.

## Pinned identity and redirect authority

`ArtifactRoute::new` selects a declared artifact from the pinned manifest before
parsing its source URL. There is no arbitrary initial-URL constructor.

Hub redirects must retain this artifact's immutable model/revision/path, either
at its original resolve path or the exact `/api/resolve-cache/models/` path.
For a declared nested artifact, the policy also matches the exact cache path
whose artifact-relative slash is encoded as `%2F`, as observed for the pooling
config. It derives that variant from the pinned artifact only; it does not decode
incoming paths or authorize arbitrary encoded separators, traversal, double
encoding, other artifacts or other revisions. Signed query preservation is unchanged.
The weight may redirect from the Hub to the explicitly supported regional host
`us.aws.cdn.hf.co`, under its constrained Xet object path. The CDN is terminal:
no subsequent redirect is accepted. Config files cannot use this CDN route.

Other hosts and regions fail closed. A host change requires review rather than
wildcard expansion. This deliberately limits compatibility to the observed route;
it does not model every Hugging Face deployment.

Only HTTPS/default TLS port is accepted. Userinfo, fragments, malformed or
ambiguous authorities, raw whitespace/control/backslash and unsupported paths
are rejected. URL normalization must not turn an empty userinfo or missing
authority into acceptance. Raw Location and resolved URLs are bounded to 16 KiB.
At most four redirects are accepted, and repeated URLs are rejected as cycles.
Failed transitions leave the route unchanged.

Opaque query bytes are preserved, not decoded, sorted or reconstructed. The
route has no `Debug`/`Display` or URL getter; policy errors carry only static
variants. These choices reduce accidental signed-query disclosure, but later
transport logging and error handling still require independent review.

The route-only unit depends directly on the already-locked URL parser.
A focused regression demonstrated that the workspace HTTP defaults enabled a
second Rustls crypto provider and made existing Neo4j TLS initialization panic.
Removing that unnecessary dependency restored initialization. The HTTP unit now
uses `rustls-no-provider` and a matching preconfigured Rustls client with an
explicit per-client ring provider. It does not install a process-global provider
or change root dependency features. The isolated regression still passes and does
not test a TLS connection, certificate validation or live Neo4j.

## HTTPS acquisition

`ArtifactHttp::new` loads the host's native trust store and fails closed if no
usable roots are available. Host-configured native-store environment overrides
remain part of trusted host configuration; no public API accepts certificates,
credentials, proxies, DNS overrides or arbitrary initial URLs.

The client uses HTTPS with certificate and hostname verification, manual redirects,
no retries, no proxy/referrer, no decompression and no client identity. HTTP/1 and
disabled idle pooling keep the tested cancellation behavior explicit. Connect,
read-idle and total limits are 10, 15 and 120 seconds. The outer total deadline
spans all redirects. Synchronous copying/hashing is cooperative: the deadline is
checked again before returning verified success, not a hard CPU preemption bound.

Only terminal 200 responses are accepted. Parsed final headers are bounded to
64 entries and 32 KiB of names/values. The policy rejects Content-Range,
nonidentity Content-Encoding, duplicate or conflicting framing, and unsupported
transfer coding. Chunked or close-delimited responses still require exact decoded
body length and digest. The body buffer never accumulates beyond the trusted
manifest size; incoming parser/network chunks and trailing wire bytes are distinct
from that decoded-body bound.

Header limits occur after the library parser and do not describe every wire,
informational-response or trailer allocation. The client trusts the host OS
resolver and PKI; exact authority validation is not public-IP filtering.

Dropping an in-flight fetch stops body consumption without an application-owned
detached worker or disk mutation. A dependency's active blocking DNS lookup may
finish after cancellation. Local tests observe connection closure within a bounded
interval, not instantaneous termination of every dependency operation.

## Evidence and unfinished work

Deterministic tests cover the route policy without remote network. Separate sanitized
HEAD probes observed a configuration redirect to the immutable Hub cache and a
weight redirect to the regional CDN. The probes did not contact the CDN and do
not establish GET behavior, TLS enforcement or downloader safety.

Real local TLS fixtures test valid chains/names, untrusted or wrong-name rejection
with decrypted-byte counters, manual redirects, framing/header limits, body size,
digest, deadlines, cancellation and staging handoff. Test-only ephemeral roots and
loopback resolution use the same builder without disabling verification. These
fixtures do not establish compatibility with the current real Hub/CDN.

The next acquisition run must verify the actual immutable model files and record
their bytes/hashes. Existing service quotas, maintenance and crash-recovery gates
remain open; no inference, C1 or parity claim follows from these transport tests.

No weights, encoder, C1, TCK, service-exposure or global parity gate is approved
by this URL-policy unit.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline artifact_route::
```
