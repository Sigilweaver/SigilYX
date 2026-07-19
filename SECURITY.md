# Security policy

## Supported versions

Only the latest minor release of SigilYX (the `sigilyx` crate and the
`sigilyx` PyPI package) is supported with security fixes.

| Version | Supported |
| ------- | --------- |
| 0.3.x   | Yes       |
| < 0.3   | No        |

## Reporting a vulnerability

Please report security vulnerabilities privately via
[GitHub Security Advisories](https://github.com/Sigilweaver/SigilYX/security/advisories/new).

Do **not** open a public issue for security reports. We will
acknowledge within 7 days and aim to publish a fix or mitigation
within 30 days for confirmed issues.

## Scope

In scope:

- Memory-safety bugs in the YXDB reader, writer, FFI surface, or
  Python bindings.
- Arbitrary file read/write triggered by a malformed `.yxdb` file.
- Crashes or undefined behaviour that can be triggered by a
  third-party-supplied YXDB input.
- Supply-chain integrity issues affecting published crates,
  wheels, or release artifacts.

Out of scope:

- Incorrect decoding of fields that fall outside the documented
  format scope (for example, experimental E2 types behind opt-in
  flags): please open a normal issue, these are correctness
  problems rather than security issues.
- Denial-of-service from intentionally oversized inputs that the
  parser correctly rejects.
- Vulnerabilities in third-party crates: please report those
  upstream.

## Disclosure

Coordinated disclosure is preferred. Once a fix is released, the
advisory will be made public and credited to the reporter unless
they request anonymity.
