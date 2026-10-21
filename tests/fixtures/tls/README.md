# TLS distrust fixtures

These are synthetic certificates, valid from 2025-01-01 through 2035-01-01,
issued beneath an unrelated, test-only root. The tests use a fixed verification
time and the DNS name `localhost`; no network or external certificate service is
required. Only the synthetic leaf keys needed by the local TLS servers are
retained as clearly named `*-test-key.pem` fixtures. They are public test data,
never credentials for a real service. Root and intermediate private keys are
not retained.

The `distrusted` intermediate uses the already-public weak P-384 scalar 156.
Its public key matches rustls-webpki's `EMDVTLSCAG3B.spki.der` denylist entry.
The `reissued` intermediate uses the same key with a different subject and
serial number. Each has a separately issued localhost leaf. The `valid`
intermediate uses an ordinary randomly generated key and supplies the positive
control.

The test root is explicitly trusted so that removing an eMudhra root from the
bundled root store cannot mask a missing key-level verifier fix. Both distrusted
chains must fail with `CertDistrusted`, including when the compromised key is
reissued beneath this otherwise trusted root. The ordinary chain must succeed.

`scripts/tls-regression.py` exercises the application's real JWKS HTTPS path
against those same chains, using an isolated child process's `SSL_CERT_FILE`.
It also checks that a hostname mismatch still fails. The child must reject
the compromised keys before starting the HTTP listener, while accepting a
normal chain under an administrator-supplied trust root.

Upstream fix and public key provenance:
https://github.com/rustls/webpki/pull/546
