# Security policy

Crosswire is a security testing tool, and the tool itself has to be safe to run. If you find a vulnerability in Crosswire — in the parser, the storage format, the (future) proxy, API or UI — please report it privately.

## Reporting

- Use **GitHub Security Advisories**: open a private report at <https://github.com/kalash33/crosswire/security/advisories/new>.
- Do not open a public issue for a vulnerability.
- Include: the affected crate and version (or commit), a minimal reproducer (a raw HTTP message or corpus case is ideal), and the impact you believe it has.

## What to expect

- Acknowledgement within **72 hours**.
- A fix or a documented mitigation within **90 days** of the report, sooner for anything that lets a target site or a hostile input crash, exhaust or take over the tool.
- Credit in the release notes if you want it.

## Scope

In scope: anything a hostile HTTP peer, a hostile project file or import, or a malicious page rendered in the tool can do to the person running Crosswire — crashes, unbounded memory or CPU, data-fidelity violations (bytes shown that differ from bytes sent or received), information disclosure, privilege escalation, and later the API, CA handling and UI.

Out of scope: findings in the *targets* you test with Crosswire, and intentional behaviour such as the ability to send malformed HTTP (that is the tool's purpose).

## Verifying releases

Every release asset carries a GitHub build-provenance attestation. Verify a download with:

```
gh attestation verify <file> --repo kalash33/crosswire
```

## Supported versions

Pre-1.0: only the `main` branch and the latest tagged release receive fixes.
