# TLS test fixture

Public, self-signed loopback certificate and disposable private key used only by
controlled tests. These are not credentials for any deployed service. The
certificate is valid from October 2026 to September 2126, with IP SAN 127.0.0.1.
Production clients must reject it. A test-only client explicitly trusts it to
verify HTTPS-to-HTTP redirect rejection without weakening production TLS.
