# Security reporting

For a vulnerability, [submit a private report on GitHub](https://github.com/DharanesshMD/workspace-knowledge/security/advisories/new). If GitHub does not offer the form, contact the maintainer through an established private channel. Keep sensitive details out of public issues and pull requests.

Include the affected revision, platform, client version, impact and a minimal reproduction using synthetic data. Omit credentials, personal conversations, customer records and production database contents.

## Evaluation scope

The current source on `main` targets evaluation with a Linux Docker server and macOS developer tools. The latest tagged prerelease is [v0.1.0-alpha.3](https://github.com/DharanesshMD/workspace-knowledge/releases/tag/v0.1.0-alpha.3). Both are evaluation software. Production identity-provider qualification, clean-machine distribution, Apple signing and notarization, a real-team pilot and independent security review remain open.

On `main`, eligible assistant-written cards can publish immediately after screening when a developer shares them. Operators can require review with `WK_AUTO_PUBLISH_PRODUCTS=none`. Capture support depends on the exact client version and surface. See the [readiness ledger](docs/operations/production-readiness.md).

## Dependency audit

On November 23, 2026, the source candidate updated rustix to 1.1.6 and url to 2.5.9. The locked Cargo audit reported zero vulnerabilities and zero unsoundness warnings. Five maintenance warnings remain for chrono, rustls-pemfile, zbus_names, zvariant and zvariant_derive. CI repeats the dependency audit when `main` changes.

See the [threat model](docs/security/threat-model.md), [RSA remediation](docs/security/rsa-runtime-disposition.md) and [restore boundaries](docs/operations/restore.md).
