# Public test-only TLS material

These certificates and the **public, deliberately committed test private key**
are only for the local loopback MCP TLS integration test. They are not secrets,
production identities or credentials. Never use them outside tests.

The leaf certificate is signed by the fixture CA, has only IP SAN `127.0.0.1`,
and is not a CA. The test verifies rejection by default, rejection for a hostname
mismatch even after host trust, and success with explicit host-supplied CA trust.
The CA signing key was disposable and is not needed or included.
